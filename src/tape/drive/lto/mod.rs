//! Driver for LTO SCSI tapes
//!
//! This is a userspace drive implementation using SG_IO.
//!
//! Why we do not use the Linux tape driver:
//!
//! - missing features (MAM, Encryption, ...)
//!
//! - strange permission handling - only root (or CAP_SYS_RAWIO) can
//!   do SG_IO (SYS_RAW_IO)
//!
//! - unability to detect EOT (you just get EIO)

use std::fs::File;
use std::os::unix::io::{AsRawFd, FromRawFd, RawFd};

use anyhow::{bail, format_err, Error};

use pbs_tape::sg_tape::drive_get_encryption;
use proxmox_uuid::Uuid;

use pbs_api_types::{
    Fingerprint, Lp17VolumeStatistics, LtoDriveAndMediaStatus, LtoTapeDrive, MamAttribute,
};
use pbs_key_config::KeyConfig;
use pbs_tape::{
    sg_tape::{SgTape, TapeAlertFlags},
    BlockReadError, MediaContentHeader, TapeRead, TapeWrite,
};
use proxmox_sys::command::run_command;

use crate::tape::{
    drive::TapeDriver,
    file_formats::{MediaSetLabel, PROXMOX_BACKUP_MEDIA_SET_LABEL_MAGIC_1_0},
};

impl Drop for LtoTapeHandle {
    fn drop(&mut self) {
        // always unload the encryption key when the handle is dropped for security
        // but only log an error if we set one in the first place
        if let Err(err) = self.set_encryption(None) {
            if self.encryption_key_loaded {
                log::error!("could not unload encryption key from drive: {err}");
            }
        }
    }
}

/// Lto Tape device handle
pub struct LtoTapeHandle {
    sg_tape: SgTape,
    encryption_key_loaded: bool,
}

impl LtoTapeHandle {
    /// Creates a new instance
    pub fn new(file: File) -> Result<Self, Error> {
        let sg_tape = SgTape::new(file)?;
        Ok(Self {
            sg_tape,
            encryption_key_loaded: false,
        })
    }

    /// Open a tape device
    ///
    /// since this calls [SgTape::open_lto_drive], it does some internal checks.
    /// See [SgTape] docs for details.
    pub fn open_lto_drive(config: &LtoTapeDrive) -> Result<Self, Error> {
        let sg_tape = SgTape::open_lto_drive(config)?;

        let handle = Self {
            sg_tape,
            encryption_key_loaded: false,
        };

        Ok(handle)
    }

    /// Get Tape and Media status
    pub fn get_drive_and_media_status(&mut self) -> Result<LtoDriveAndMediaStatus, Error> {
        self.sg_tape.get_drive_and_media_status()
    }

    pub fn forward_space_count_files(&mut self, count: usize) -> Result<(), Error> {
        self.sg_tape.space_filemarks(count.try_into()?)
    }

    pub fn backward_space_count_files(&mut self, count: usize) -> Result<(), Error> {
        self.sg_tape.space_filemarks(-count.try_into()?)
    }

    /// Position the tape after filemark count. Count 0 means BOT.
    pub fn locate_file(&mut self, position: u64) -> Result<(), Error> {
        self.sg_tape.locate_file(position)
    }

    /// Read Cartridge Memory (MAM Attributes)
    pub fn cartridge_memory(&mut self) -> Result<Vec<MamAttribute>, Error> {
        self.sg_tape.cartridge_memory()
    }

    /// Read Volume Statistics
    pub fn volume_statistics(&mut self) -> Result<Lp17VolumeStatistics, Error> {
        self.sg_tape.volume_statistics()
    }

    /// Returns if a medium is present
    pub fn medium_present(&mut self) -> bool {
        self.sg_tape.test_unit_ready().is_ok()
    }
}

impl TapeDriver for LtoTapeHandle {
    fn sync(&mut self) -> Result<(), Error> {
        self.sg_tape.sync()?;
        Ok(())
    }

    /// Go to the end of the recorded media (for appending files).
    fn move_to_eom(&mut self, write_missing_eof: bool) -> Result<(), Error> {
        self.sg_tape.move_to_eom(write_missing_eof)
    }

    fn move_to_last_file(&mut self) -> Result<(), Error> {
        self.move_to_eom(false)?;

        self.sg_tape.check_filemark()?;

        let pos = self.current_file_number()?;

        if pos == 0 {
            bail!("move_to_last_file failed - media contains no data");
        }

        if pos == 1 {
            self.rewind()?;
            return Ok(());
        }

        self.backward_space_count_files(2)?;
        self.forward_space_count_files(1)?;

        Ok(())
    }

    fn move_to_file(&mut self, file: u64) -> Result<(), Error> {
        self.locate_file(file)
    }

    fn rewind(&mut self) -> Result<(), Error> {
        self.sg_tape.rewind()
    }

    fn current_file_number(&mut self) -> Result<u64, Error> {
        self.sg_tape.current_file_number()
    }

    fn format_media(&mut self, fast: bool) -> Result<(), Error> {
        self.sg_tape.format_media(fast)
    }

    fn read_next_file<'a>(&'a mut self) -> Result<Box<dyn TapeRead + 'a>, BlockReadError> {
        let reader = self.sg_tape.open_reader()?;
        let handle: Box<dyn TapeRead> = Box::new(reader);
        Ok(handle)
    }

    fn write_file<'a>(&'a mut self) -> Result<Box<dyn TapeWrite + 'a>, std::io::Error> {
        let handle = self.sg_tape.open_writer();
        Ok(Box::new(handle))
    }

    fn write_media_set_label(
        &mut self,
        media_set_label: &MediaSetLabel,
        key_config: Option<&KeyConfig>,
    ) -> Result<(), Error> {
        let file_number = self.current_file_number()?;
        if file_number != 1 {
            self.rewind()?;
            self.forward_space_count_files(1)?; // skip label
        }

        let file_number = self.current_file_number()?;
        if file_number != 1 {
            bail!(
                "write_media_set_label failed - got wrong file number ({} != 1)",
                file_number
            );
        }

        self.set_encryption(None)?;

        {
            // limit handle scope
            let mut handle = self.write_file()?;

            let mut value = serde_json::to_value(media_set_label)?;
            if media_set_label.encryption_key_fingerprint.is_some() {
                match key_config {
                    Some(key_config) => {
                        value["key-config"] = serde_json::to_value(key_config)?;
                    }
                    None => {
                        bail!("missing encryption key config");
                    }
                }
            }

            let raw = serde_json::to_string_pretty(&value)?;

            let header =
                MediaContentHeader::new(PROXMOX_BACKUP_MEDIA_SET_LABEL_MAGIC_1_0, raw.len() as u32);
            handle.write_header(&header, raw.as_bytes())?;
            handle.finish(false)?;
        }

        self.sync()?; // sync data to tape

        let encrypt_fingerprint = media_set_label
            .encryption_key_fingerprint
            .clone()
            .map(|fp| (fp, media_set_label.uuid.clone()));

        self.set_encryption(encrypt_fingerprint)?;

        self.write_additional_attributes(None, Some(media_set_label.pool.clone()));

        Ok(())
    }

    /// Rewind and put the drive off line (Eject media).
    fn eject_media(&mut self) -> Result<(), Error> {
        self.sg_tape.eject()
    }

    /// Read Tape Alert Flags
    fn tape_alert_flags(&mut self) -> Result<TapeAlertFlags, Error> {
        self.sg_tape.tape_alert_flags()
    }

    /// Set or clear encryption key
    ///
    /// Note: Only 'root' can read secret encryption keys, so we need
    /// to spawn setuid binary 'sg-tape-cmd'.
    fn set_encryption(
        &mut self,
        key_fingerprint: Option<(Fingerprint, Uuid)>,
    ) -> Result<(), Error> {
        if let Some((fingerprint, uuid)) = key_fingerprint {
            let fingerprint = fingerprint.signature();
            let output = run_sg_tape_cmd(
                "encryption",
                &["--fingerprint", &fingerprint, "--uuid", &uuid.to_string()],
                self.sg_tape.file_mut().as_raw_fd(),
            )?;
            self.encryption_key_loaded = true;
            let result: Result<(), String> = serde_json::from_str(&output)?;
            result.map_err(|err| format_err!("{}", err))
        } else {
            self.sg_tape.set_encryption(None)
        }
    }

    fn assert_encryption_mode(&mut self, encryption_wanted: bool) -> Result<(), Error> {
        let encryption_set = drive_get_encryption(self.sg_tape.file_mut())?;
        if encryption_wanted != encryption_set {
            bail!("Set encryption mode not what was desired (set: {encryption_set}, wanted: {encryption_wanted})");
        }
        Ok(())
    }

    fn get_volume_statistics(&mut self) -> Result<pbs_api_types::Lp17VolumeStatistics, Error> {
        self.volume_statistics()
    }

    fn write_additional_attributes(&mut self, label: Option<String>, pool: Option<String>) {
        self.sg_tape.write_mam_attributes(label, pool)
    }
}

fn run_sg_tape_cmd(subcmd: &str, args: &[&str], fd: RawFd) -> Result<String, Error> {
    let mut command =
        std::process::Command::new("/usr/lib/x86_64-linux-gnu/proxmox-backup/sg-tape-cmd");
    command.args([subcmd]);
    command.args(["--stdin"]);
    command.args(args);
    let device_fd = nix::unistd::dup(fd)?;
    command.stdin(unsafe { std::process::Stdio::from_raw_fd(device_fd) });
    run_command(command, None)
}
