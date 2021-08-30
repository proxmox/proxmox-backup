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

mod sg_tape;
pub use sg_tape::*;

use std::fs::{OpenOptions, File};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::{AsRawFd, FromRawFd, RawFd};
use std::convert::TryInto;

use anyhow::{bail, format_err, Error};
use nix::fcntl::{fcntl, FcntlArg, OFlag};

use proxmox::{
    tools::Uuid,
    sys::error::SysResult,
};

use pbs_api_types::Fingerprint;
use pbs_datastore::key_derivation::KeyConfig;

use crate::{
    config,
    tools::run_command,
    api2::types::{
        MamAttribute,
        LtoDriveAndMediaStatus,
        LtoTapeDrive,
        Lp17VolumeStatistics,
    },
    tape::{
        TapeRead,
        TapeWrite,
        BlockReadError,
        drive::{
            TapeDriver,
        },
        file_formats::{
            PROXMOX_BACKUP_MEDIA_SET_LABEL_MAGIC_1_0,
            MediaSetLabel,
            MediaContentHeader,
        },
    },
};

impl LtoTapeDrive {

    /// Open a tape device
    ///
    /// This does additional checks:
    ///
    /// - check if it is a non-rewinding tape device
    /// - check if drive is ready (tape loaded)
    /// - check block size
    /// - for autoloader only, try to reload ejected tapes
    pub fn open(&self) -> Result<LtoTapeHandle, Error> {

        proxmox::try_block!({
            let file = open_lto_tape_device(&self.path)?;

            let mut handle = LtoTapeHandle::new(file)?;

            if !handle.sg_tape.test_unit_ready().is_ok() {
                // for autoloader only, try to reload ejected tapes
                if self.changer.is_some() {
                    let _ = handle.sg_tape.load(); // just try, ignore error
                }
            }

            handle.sg_tape.wait_until_ready()?;

            handle.set_default_options()?;

            Ok(handle)
        }).map_err(|err: Error| format_err!("open drive '{}' ({}) failed - {}", self.name, self.path, err))
    }
}

/// Lto Tape device handle
pub struct LtoTapeHandle {
    sg_tape: SgTape,
}

impl LtoTapeHandle {

    /// Creates a new instance
    pub fn new(file: File) -> Result<Self, Error> {
        let sg_tape = SgTape::new(file)?;
        Ok(Self { sg_tape })
    }

    /// Set all options we need/want
    pub fn set_default_options(&mut self) -> Result<(), Error> {

        let compression = Some(true);
        let block_length = Some(0); // variable length mode
        let buffer_mode = Some(true); // Always use drive buffer

        self.set_drive_options(compression, block_length, buffer_mode)?;

        Ok(())
    }

    /// Set driver options
    pub fn set_drive_options(
        &mut self,
        compression: Option<bool>,
        block_length: Option<u32>,
        buffer_mode: Option<bool>,
    ) -> Result<(), Error> {
        self.sg_tape.set_drive_options(compression, block_length, buffer_mode)
    }

    /// Write a single EOF mark without flushing buffers
    pub fn write_filemarks(&mut self, count: usize) -> Result<(), std::io::Error> {
        self.sg_tape.write_filemarks(count, false)
    }

    /// Get Tape and Media status
    pub fn get_drive_and_media_status(&mut self) -> Result<LtoDriveAndMediaStatus, Error>  {

        let drive_status = self.sg_tape.read_drive_status()?;

        let alert_flags = self.tape_alert_flags()
            .map(|flags| format!("{:?}", flags))
            .ok();

        let mut status = LtoDriveAndMediaStatus {
            vendor: self.sg_tape.info().vendor.clone(),
            product: self.sg_tape.info().product.clone(),
            revision: self.sg_tape.info().revision.clone(),
            blocksize: drive_status.block_length,
            compression: drive_status.compression,
            buffer_mode: drive_status.buffer_mode,
            density: drive_status.density_code.try_into()?,
            alert_flags,
            write_protect: None,
            file_number: None,
            block_number: None,
            manufactured: None,
            bytes_read: None,
            bytes_written: None,
            medium_passes: None,
            medium_wearout: None,
            volume_mounts: None,
        };

        if self.sg_tape.test_unit_ready().is_ok() {

            if drive_status.write_protect {
                status.write_protect = Some(drive_status.write_protect);
            }

            let position = self.sg_tape.position()?;

            status.file_number = Some(position.logical_file_id);
            status.block_number = Some(position.logical_object_number);

            if let Ok(mam) = self.cartridge_memory() {

                let usage = mam_extract_media_usage(&mam)?;

                status.manufactured = Some(usage.manufactured);
                status.bytes_read = Some(usage.bytes_read);
                status.bytes_written = Some(usage.bytes_written);

                if let Ok(volume_stats) = self.volume_statistics() {

                    let passes = std::cmp::max(
                        volume_stats.beginning_of_medium_passes,
                        volume_stats.middle_of_tape_passes,
                    );

                    // assume max. 16000 medium passes
                    // see: https://en.wikipedia.org/wiki/Linear_Tape-Open
                    let wearout: f64 = (passes as f64)/(16000.0 as f64);

                    status.medium_passes = Some(passes);
                    status.medium_wearout = Some(wearout);

                    status.volume_mounts = Some(volume_stats.volume_mounts);
                }
            }
        }

        Ok(status)
    }

    pub fn forward_space_count_files(&mut self, count: usize) -> Result<(), Error> {
        self.sg_tape.space_filemarks(count.try_into()?)
    }

    pub fn backward_space_count_files(&mut self, count: usize) -> Result<(), Error> {
        self.sg_tape.space_filemarks(-count.try_into()?)
    }

    pub fn forward_space_count_records(&mut self, count: usize) -> Result<(), Error> {
        self.sg_tape.space_blocks(count.try_into()?)
    }

    pub fn backward_space_count_records(&mut self, count: usize) -> Result<(), Error> {
        self.sg_tape.space_blocks(-count.try_into()?)
    }

    /// Position the tape after filemark count. Count 0 means BOT.
    pub fn locate_file(&mut self, position: u64) ->  Result<(), Error> {
        self.sg_tape.locate_file(position)
    }

    pub fn erase_media(&mut self, fast: bool) -> Result<(), Error> {
        self.sg_tape.erase_media(fast)
    }

    pub fn load(&mut self) ->  Result<(), Error> {
        self.sg_tape.load()
    }

    /// Read Cartridge Memory (MAM Attributes)
    pub fn cartridge_memory(&mut self) -> Result<Vec<MamAttribute>, Error> {
        self.sg_tape.cartridge_memory()
     }

    /// Read Volume Statistics
    pub fn volume_statistics(&mut self) -> Result<Lp17VolumeStatistics, Error> {
        self.sg_tape.volume_statistics()
    }

    /// Lock the drive door
    pub fn lock(&mut self) -> Result<(), Error>  {
        self.sg_tape.set_medium_removal(false)
            .map_err(|err| format_err!("lock door failed - {}", err))
    }

    /// Unlock the drive door
    pub fn unlock(&mut self) -> Result<(), Error>  {
        self.sg_tape.set_medium_removal(true)
            .map_err(|err| format_err!("unlock door failed - {}", err))
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
            bail!("write_media_set_label failed - got wrong file number ({} != 1)", file_number);
        }

        self.set_encryption(None)?;

        { // limit handle scope
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

            let header = MediaContentHeader::new(PROXMOX_BACKUP_MEDIA_SET_LABEL_MAGIC_1_0, raw.len() as u32);
            handle.write_header(&header, raw.as_bytes())?;
            handle.finish(false)?;
        }

        self.sync()?; // sync data to tape

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

        if nix::unistd::Uid::effective().is_root() {

            if let Some((ref key_fingerprint, ref uuid)) = key_fingerprint {

                let (key_map, _digest) = config::tape_encryption_keys::load_keys()?;
                match key_map.get(key_fingerprint) {
                    Some(item) => {

                        // derive specialized key for each media-set

                        let mut tape_key = [0u8; 32];

                        let uuid_bytes: [u8; 16] = uuid.as_bytes().clone();

                        openssl::pkcs5::pbkdf2_hmac(
                            &item.key,
                            &uuid_bytes,
                            10,
                            openssl::hash::MessageDigest::sha256(),
                            &mut tape_key)?;

                        return self.sg_tape.set_encryption(Some(tape_key));
                    }
                    None => bail!("unknown tape encryption key '{}'", key_fingerprint),
                }
            } else {
                return self.sg_tape.set_encryption(None);
            }
        }

        let output = if let Some((fingerprint, uuid)) = key_fingerprint {
            let fingerprint = pbs_tools::format::as_fingerprint(fingerprint.bytes());
            run_sg_tape_cmd("encryption", &[
                "--fingerprint", &fingerprint,
                "--uuid", &uuid.to_string(),
            ], self.sg_tape.file_mut().as_raw_fd())?
        } else {
            run_sg_tape_cmd("encryption", &[], self.sg_tape.file_mut().as_raw_fd())?
        };
        let result: Result<(), String> = serde_json::from_str(&output)?;
        result.map_err(|err| format_err!("{}", err))
    }
}

/// Check for correct Major/Minor numbers
pub fn check_tape_is_lto_tape_device(file: &File) -> Result<(), Error> {

    let stat = nix::sys::stat::fstat(file.as_raw_fd())?;

    let devnum = stat.st_rdev;

    let major = unsafe { libc::major(devnum) };
    let _minor = unsafe { libc::minor(devnum) };

    if major == 9 {
        bail!("not a scsi-generic tape device (cannot use linux tape devices)");
    }

    if major != 21 {
        bail!("not a scsi-generic tape device");
    }

    Ok(())
}

/// Opens a Lto tape device
///
/// The open call use O_NONBLOCK, but that flag is cleard after open
/// succeeded. This also checks if the device is a non-rewinding tape
/// device.
pub fn open_lto_tape_device(
    path: &str,
) -> Result<File, Error> {

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)?;

    // clear O_NONBLOCK from now on.

    let flags = fcntl(file.as_raw_fd(), FcntlArg::F_GETFL)
        .into_io_result()?;

    let mut flags = OFlag::from_bits_truncate(flags);
    flags.remove(OFlag::O_NONBLOCK);

    fcntl(file.as_raw_fd(), FcntlArg::F_SETFL(flags))
        .into_io_result()?;

    check_tape_is_lto_tape_device(&file)
        .map_err(|err| format_err!("device type check {:?} failed - {}", path, err))?;

    Ok(file)
}

fn run_sg_tape_cmd(subcmd: &str, args: &[&str], fd: RawFd) -> Result<String, Error> {
    let mut command = std::process::Command::new(
        "/usr/lib/x86_64-linux-gnu/proxmox-backup/sg-tape-cmd");
    command.args(&[subcmd]);
    command.args(&["--stdin"]);
    command.args(args);
    let device_fd = nix::unistd::dup(fd)?;
    command.stdin(unsafe { std::process::Stdio::from_raw_fd(device_fd)});
    run_command(command, None)
}
