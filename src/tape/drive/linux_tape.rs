//! Driver for Linux SCSI tapes

use std::fs::{OpenOptions, File};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::{AsRawFd, FromRawFd, RawFd};
use std::convert::TryFrom;

use anyhow::{bail, format_err, Error};
use nix::fcntl::{fcntl, FcntlArg, OFlag};

use proxmox::sys::error::SysResult;
use proxmox::tools::Uuid;

use crate::{
    config,
    backup::{
        Fingerprint,
        KeyConfig,
    },
    tools::run_command,
    api2::types::{
        TapeDensity,
        MamAttribute,
        LinuxDriveAndMediaStatus,
    },
    tape::{
        TapeRead,
        TapeWrite,
        drive::{
            linux_mtio::*,
            LinuxTapeDrive,
            TapeDriver,
            TapeAlertFlags,
            Lp17VolumeStatistics,
            read_mam_attributes,
            mam_extract_media_usage,
            read_tape_alert_flags,
            read_volume_statistics,
            set_encryption,
        },
        file_formats::{
            PROXMOX_TAPE_BLOCK_SIZE,
            PROXMOX_BACKUP_MEDIA_SET_LABEL_MAGIC_1_0,
            MediaSetLabel,
            MediaContentHeader,
            BlockedReader,
            BlockedWriter,
        },
    },
};

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

/// Linux tape drive status
#[derive(Debug)]
pub struct LinuxDriveStatus {
    /// Size 0 is variable block size mode (default)
    pub blocksize: u32,
    /// Drive status flags
    pub status: GMTStatusFlags,
    /// Tape densitiy code (if drive media loaded)
    pub density: Option<TapeDensity>,
    /// Current file position if known (or -1)
    pub file_number: Option<u32>,
    /// Current block number if known (or -1)
    pub block_number: Option<u32>,
}

impl LinuxDriveStatus {
    pub fn tape_is_ready(&self) -> bool {
        self.status.contains(GMTStatusFlags::ONLINE) &&
            !self.status.contains(GMTStatusFlags::DRIVE_OPEN)
    }
}

impl LinuxTapeDrive {

    /// Open a tape device
    ///
    /// This does additional checks:
    ///
    /// - check if it is a non-rewinding tape device
    /// - check if drive is ready (tape loaded)
    /// - check block size
    /// - for autoloader only, try to reload ejected tapes
    pub fn open(&self) -> Result<LinuxTapeHandle, Error> {

        proxmox::try_block!({
            let file = open_linux_tape_device(&self.path)?;

            let mut handle = LinuxTapeHandle::new(file);

            let mut drive_status = handle.get_drive_status()?;

            if !drive_status.tape_is_ready() {
                // for autoloader only, try to reload ejected tapes
                if self.changer.is_some() {
                    let _ = handle.mtload(); // just try, ignore error
                    drive_status = handle.get_drive_status()?;
                }
            }

            if !drive_status.tape_is_ready() {
                bail!("tape not ready (no tape loaded)");
            }

            if drive_status.blocksize == 0 {
                // device is variable block size - OK
            } else if drive_status.blocksize != PROXMOX_TAPE_BLOCK_SIZE as u32 {
                eprintln!("device is in fixed block size mode with wrong size ({} bytes)", drive_status.blocksize);
                eprintln!("trying to set variable block size mode...");
                if handle.set_block_size(0).is_err() {
                    bail!("set variable block size mod failed - device uses wrong blocksize.");
                }
            } else {
                // device is in fixed block size mode with correct block size
            }

            // Only root can set driver options, so we cannot
            // handle.set_default_options()?;

            Ok(handle)
        }).map_err(|err| format_err!("open drive '{}' ({}) failed - {}", self.name, self.path, err))
    }
}

/// Linux Tape device handle
pub struct LinuxTapeHandle {
    file: File,
    //_lock: File,
}

impl LinuxTapeHandle {

    /// Creates a new instance
    pub fn new(file: File) -> Self {
        Self { file }
    }

    /// Set all options we need/want
    pub fn set_default_options(&self) -> Result<(), Error> {

        let mut opts = SetDrvBufferOptions::empty();

        // fixme: ? man st(4) claims we need to clear this for reliable multivolume
        opts.set(SetDrvBufferOptions::BUFFER_WRITES, true);

        // fixme: ?man st(4) claims we need to clear this for reliable multivolume
        opts.set(SetDrvBufferOptions::ASYNC_WRITES, true);

        opts.set(SetDrvBufferOptions::READ_AHEAD, true);

        self.set_drive_buffer_options(opts)
    }

    /// call MTSETDRVBUFFER to set boolean options
    ///
    /// Note: this uses MT_ST_BOOLEANS, so missing options are cleared!
    pub fn set_drive_buffer_options(&self, opts: SetDrvBufferOptions) -> Result<(), Error> {

        let cmd = mtop {
            mt_op: MTCmd::MTSETDRVBUFFER,
            mt_count: (SetDrvBufferCmd::MT_ST_BOOLEANS as i32) | opts.bits(),
        };
        unsafe {
            mtioctop(self.file.as_raw_fd(), &cmd)
        }.map_err(|err| format_err!("MTSETDRVBUFFER options failed - {}", err))?;

        Ok(())
    }

    /// call MTSETDRVBUFFER to set boolean options
    ///
    /// Note: this uses MT_ST_SETBOOLEANS
    pub fn drive_buffer_set_options(&self, opts: SetDrvBufferOptions) -> Result<(), Error> {

        let cmd = mtop {
            mt_op: MTCmd::MTSETDRVBUFFER,
            mt_count: (SetDrvBufferCmd::MT_ST_SETBOOLEANS as i32) | opts.bits(),
        };
        unsafe {
            mtioctop(self.file.as_raw_fd(), &cmd)
        }.map_err(|err| format_err!("MTSETDRVBUFFER options failed - {}", err))?;

        Ok(())
    }

    /// call MTSETDRVBUFFER to clear boolean options
    pub fn drive_buffer_clear_options(&self, opts: SetDrvBufferOptions) -> Result<(), Error> {

        let cmd = mtop {
            mt_op: MTCmd::MTSETDRVBUFFER,
            mt_count: (SetDrvBufferCmd::MT_ST_CLEARBOOLEANS as i32) | opts.bits(),
        };
        unsafe {
            mtioctop(self.file.as_raw_fd(), &cmd)
        }.map_err(|err| format_err!("MTSETDRVBUFFER options failed - {}", err))?;

        Ok(())
    }

    /// This flushes the driver's buffer as a side effect. Should be
    /// used before reading status with MTIOCGET.
    fn mtnop(&self) -> Result<(), Error> {

        let cmd = mtop { mt_op: MTCmd::MTNOP, mt_count: 1, };

        unsafe {
            mtioctop(self.file.as_raw_fd(), &cmd)
        }.map_err(|err| format_err!("MTNOP failed - {}", err))?;

        Ok(())
    }

    pub fn mtop(&mut self, mt_op: MTCmd, mt_count: i32, msg: &str) -> Result<(), Error> {
        let cmd = mtop { mt_op, mt_count };

        unsafe {
            mtioctop(self.file.as_raw_fd(), &cmd)
        }.map_err(|err| format_err!("{} failed (count {}) - {}", msg, mt_count, err))?;

        Ok(())
    }

    pub fn mtload(&mut self) -> Result<(), Error> {

        let cmd = mtop { mt_op: MTCmd::MTLOAD, mt_count: 1, };

        unsafe {
            mtioctop(self.file.as_raw_fd(), &cmd)
        }.map_err(|err| format_err!("MTLOAD failed - {}", err))?;

        Ok(())
    }

    /// Set tape compression feature
    pub fn set_compression(&self, on: bool) -> Result<(), Error> {

        let cmd = mtop { mt_op: MTCmd::MTCOMPRESSION, mt_count: if on { 1 } else { 0 } };

        unsafe {
            mtioctop(self.file.as_raw_fd(), &cmd)
        }.map_err(|err| format_err!("set compression to {} failed - {}", on, err))?;

        Ok(())
    }

    /// Write a single EOF mark
    pub fn write_eof_mark(&self) -> Result<(), Error> {
        tape_write_eof_mark(&self.file)?;
        Ok(())
    }

    /// Set the drive's block length to the value specified.
    ///
    /// A block length of zero sets the drive to variable block
    /// size mode.
    pub fn set_block_size(&self, block_length: usize) -> Result<(), Error> {

        if block_length > 256*1024*1024 {
            bail!("block_length too large (> max linux scsii block length)");
        }

        let cmd = mtop { mt_op: MTCmd::MTSETBLK, mt_count: block_length as i32 };

        unsafe {
            mtioctop(self.file.as_raw_fd(), &cmd)
        }.map_err(|err| format_err!("MTSETBLK failed - {}", err))?;

        Ok(())
    }

    /// Get Tape and Media status
    pub fn get_drive_and_media_status(&mut self) -> Result<LinuxDriveAndMediaStatus, Error>  {

        let drive_status = self.get_drive_status()?;

        let options = read_tapedev_options(&self.file)?;

        let alert_flags = self.tape_alert_flags()
            .map(|flags| format!("{:?}", flags))
            .ok();

        let mut status = LinuxDriveAndMediaStatus {
            blocksize: drive_status.blocksize,
            density: drive_status.density,
            status: format!("{:?}", drive_status.status),
            options: format!("{:?}", options),
            alert_flags,
            file_number: drive_status.file_number,
            block_number: drive_status.block_number,
            manufactured: None,
            bytes_read: None,
            bytes_written: None,
            medium_passes: None,
            medium_wearout: None,
            volume_mounts: None,
        };

        if  drive_status.tape_is_ready() {

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

    /// Get Tape status/configuration with MTIOCGET ioctl
    pub fn get_drive_status(&mut self) -> Result<LinuxDriveStatus, Error> {

        let _ = self.mtnop(); // ignore errors (i.e. no tape loaded)

        let mut status = mtget::default();

        if let Err(err) = unsafe { mtiocget(self.file.as_raw_fd(), &mut status) } {
            bail!("MTIOCGET failed - {}", err);
        }

        let gmt = GMTStatusFlags::from_bits_truncate(status.mt_gstat);

        let blocksize;

        if status.mt_type == MT_TYPE_ISSCSI1 || status.mt_type == MT_TYPE_ISSCSI2 {
            blocksize = ((status.mt_dsreg & MT_ST_BLKSIZE_MASK) >> MT_ST_BLKSIZE_SHIFT) as u32;
        } else {
            bail!("got unsupported tape type {}", status.mt_type);
        }

        let density = ((status.mt_dsreg & MT_ST_DENSITY_MASK) >> MT_ST_DENSITY_SHIFT) as u8;

        Ok(LinuxDriveStatus {
            blocksize,
            status: gmt,
            density: if density != 0 {
                Some(TapeDensity::try_from(density)?)
            } else {
                None
            },
            file_number: if status.mt_fileno > 0 {
                Some(status.mt_fileno as u32)
            } else {
                None
            },
            block_number: if status.mt_blkno > 0 {
                Some(status.mt_blkno as u32)
            } else {
                None
            },
        })
    }

    /// Read Cartridge Memory (MAM Attributes)
    ///
    /// Note: Only 'root' user may run RAW SG commands, so we need to
    /// spawn setuid binary 'sg-tape-cmd'.
    pub fn cartridge_memory(&mut self) -> Result<Vec<MamAttribute>, Error> {

        if nix::unistd::Uid::effective().is_root() {
            return read_mam_attributes(&mut self.file);
        }

        let output = run_sg_tape_cmd("cartridge-memory", &[], self.file.as_raw_fd())?;
        let result: Result<Vec<MamAttribute>, String> = serde_json::from_str(&output)?;
        result.map_err(|err| format_err!("{}", err))
    }

    /// Read Volume Statistics
    ///
    /// Note: Only 'root' user may run RAW SG commands, so we need to
    /// spawn setuid binary 'sg-tape-cmd'.
    pub fn volume_statistics(&mut self) -> Result<Lp17VolumeStatistics, Error> {

        if nix::unistd::Uid::effective().is_root() {
            return read_volume_statistics(&mut self.file);
        }

        let output = run_sg_tape_cmd("volume-statistics", &[], self.file.as_raw_fd())?;
        let result: Result<Lp17VolumeStatistics, String> = serde_json::from_str(&output)?;
        result.map_err(|err| format_err!("{}", err))
    }
}


impl TapeDriver for LinuxTapeHandle {

    fn sync(&mut self) -> Result<(), Error> {

        // MTWEOF with count 0 => flush
        let cmd = mtop { mt_op: MTCmd::MTWEOF, mt_count: 0 };

        unsafe {
            mtioctop(self.file.as_raw_fd(), &cmd)
        }.map_err(|err| proxmox::io_format_err!("MT sync failed - {}", err))?;

        Ok(())
    }

    /// Go to the end of the recorded media (for appending files).
    fn move_to_eom(&mut self) -> Result<(), Error> {

        let cmd = mtop { mt_op: MTCmd::MTEOM, mt_count: 1, };

        unsafe {
            mtioctop(self.file.as_raw_fd(), &cmd)
        }.map_err(|err| format_err!("MTEOM failed - {}", err))?;


        Ok(())
    }

    fn forward_space_count_files(&mut self, count: usize) -> Result<(), Error> {

        let cmd = mtop { mt_op: MTCmd::MTFSF, mt_count: i32::try_from(count)? };

        unsafe {
            mtioctop(self.file.as_raw_fd(), &cmd)
        }.map_err(|err| {
            format_err!("forward space {} files failed - {}", count, err)
        })?;

        Ok(())
    }

    fn backward_space_count_files(&mut self, count: usize) -> Result<(), Error> {

        let cmd = mtop { mt_op: MTCmd::MTBSF, mt_count: i32::try_from(count)? };

        unsafe {
            mtioctop(self.file.as_raw_fd(), &cmd)
        }.map_err(|err| {
            format_err!("backward space {} files failed - {}", count, err)
        })?;

        Ok(())
    }

    fn rewind(&mut self) -> Result<(), Error> {

        let cmd = mtop { mt_op: MTCmd::MTREW, mt_count: 1, };

        unsafe {
            mtioctop(self.file.as_raw_fd(), &cmd)
        }.map_err(|err| format_err!("tape rewind failed - {}", err))?;

        Ok(())
    }

    fn current_file_number(&mut self) -> Result<u64, Error> {
        let mut status = mtget::default();

        self.mtnop()?;

        if let Err(err) = unsafe { mtiocget(self.file.as_raw_fd(), &mut status) } {
            bail!("current_file_number MTIOCGET failed - {}", err);
        }

        if status.mt_fileno < 0 {
            bail!("current_file_number failed (got {})", status.mt_fileno);
        }
        Ok(status.mt_fileno as u64)
    }

    fn erase_media(&mut self, fast: bool) -> Result<(), Error> {

        self.rewind()?; // important - erase from BOT

        let cmd = mtop { mt_op: MTCmd::MTERASE, mt_count: if fast { 0 } else { 1 } };

        unsafe {
            mtioctop(self.file.as_raw_fd(), &cmd)
        }.map_err(|err| format_err!("MTERASE failed - {}", err))?;

        Ok(())
    }

    fn read_next_file<'a>(&'a mut self) -> Result<Option<Box<dyn TapeRead + 'a>>, std::io::Error> {
        match BlockedReader::open(&mut self.file)? {
            Some(reader) => Ok(Some(Box::new(reader))),
            None => Ok(None),
        }
    }

    fn write_file<'a>(&'a mut self) -> Result<Box<dyn TapeWrite + 'a>, std::io::Error> {

        let handle = TapeWriterHandle {
            writer: BlockedWriter::new(&mut self.file),
        };

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

        let mut handle = TapeWriterHandle {
            writer: BlockedWriter::new(&mut self.file),
        };

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

        self.sync()?; // sync data to tape

        Ok(())
    }

    /// Rewind and put the drive off line (Eject media).
    fn eject_media(&mut self) -> Result<(), Error> {
        let cmd = mtop { mt_op: MTCmd::MTOFFL, mt_count: 1 };

        unsafe {
            mtioctop(self.file.as_raw_fd(), &cmd)
        }.map_err(|err| format_err!("MTOFFL failed - {}", err))?;

        Ok(())
    }

    /// Read Tape Alert Flags
    ///
    /// Note: Only 'root' user may run RAW SG commands, so we need to
    /// spawn setuid binary 'sg-tape-cmd'.
    fn tape_alert_flags(&mut self) -> Result<TapeAlertFlags, Error> {

        if nix::unistd::Uid::effective().is_root() {
            return read_tape_alert_flags(&mut self.file);
        }

        let output = run_sg_tape_cmd("tape-alert-flags", &[], self.file.as_raw_fd())?;
        let result: Result<u64, String> = serde_json::from_str(&output)?;
        result
            .map_err(|err| format_err!("{}", err))
            .map(TapeAlertFlags::from_bits_truncate)
    }

    /// Set or clear encryption key
    ///
    /// Note: Only 'root' user may run RAW SG commands, so we need to
    /// spawn setuid binary 'sg-tape-cmd'. Also, encryption key file
    /// is only readable by root.
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

                        return set_encryption(&mut self.file, Some(tape_key));
                    }
                    None => bail!("unknown tape encryption key '{}'", key_fingerprint),
                }
            } else {
                return set_encryption(&mut self.file, None);
            }
        }

        let output = if let Some((fingerprint, uuid)) = key_fingerprint {
            let fingerprint = crate::tools::format::as_fingerprint(fingerprint.bytes());
            run_sg_tape_cmd("encryption", &[
                "--fingerprint", &fingerprint,
                "--uuid", &uuid.to_string(),
            ], self.file.as_raw_fd())?
        } else {
            run_sg_tape_cmd("encryption", &[], self.file.as_raw_fd())?
        };
        let result: Result<(), String> = serde_json::from_str(&output)?;
        result.map_err(|err| format_err!("{}", err))
    }
}

/// Write a single EOF mark without flushing buffers
fn tape_write_eof_mark(file: &File) -> Result<(), std::io::Error> {

    let cmd = mtop { mt_op: MTCmd::MTWEOFI, mt_count: 1 };

    unsafe {
        mtioctop(file.as_raw_fd(), &cmd)
    }.map_err(|err| proxmox::io_format_err!("MTWEOFI failed - {}", err))?;

    Ok(())
}

/// Check for correct Major/Minor numbers
pub fn check_tape_is_linux_tape_device(file: &File) -> Result<(), Error> {

    let stat = nix::sys::stat::fstat(file.as_raw_fd())?;

    let devnum = stat.st_rdev;

    let major = unsafe { libc::major(devnum) };
    let minor = unsafe { libc::minor(devnum) };

    if major != 9 {
        bail!("not a tape device");
    }
    if (minor & 128) == 0 {
        bail!("Detected rewinding tape. Please use non-rewinding tape devices (/dev/nstX).");
    }

    Ok(())
}

/// Opens a Linux tape device
///
/// The open call use O_NONBLOCK, but that flag is cleard after open
/// succeeded. This also checks if the device is a non-rewinding tape
/// device.
pub fn open_linux_tape_device(
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

    check_tape_is_linux_tape_device(&file)
        .map_err(|err| format_err!("device type check {:?} failed - {}", path, err))?;

    Ok(file)
}

/// Read Linux tape device options from /sys
pub fn read_tapedev_options(file: &File) -> Result<SetDrvBufferOptions, Error> {

    let stat = nix::sys::stat::fstat(file.as_raw_fd())?;

    let devnum = stat.st_rdev;

    let major = unsafe { libc::major(devnum) };
    let minor = unsafe { libc::minor(devnum) };

    let path = format!("/sys/dev/char/{}:{}/options", major, minor);

    let options = proxmox::tools::fs::file_read_firstline(&path)?;

    let options = options.trim();

    let options = match options.strip_prefix("0x") {
        Some(rest) => rest,
        None => bail!("unable to parse '{}'", path),
    };

    let options = i32::from_str_radix(&options, 16)?;

    Ok(SetDrvBufferOptions::from_bits_truncate(options))
}


/// like BlockedWriter, but writes EOF mark on finish
pub struct TapeWriterHandle<'a> {
    writer: BlockedWriter<&'a mut File>,
}

impl TapeWrite for TapeWriterHandle<'_> {

    fn write_all(&mut self, data: &[u8]) -> Result<bool, std::io::Error> {
        self.writer.write_all(data)
    }

    fn bytes_written(&self) -> usize {
        self.writer.bytes_written()
    }

    fn finish(&mut self, incomplete: bool) -> Result<bool, std::io::Error> {
        let leof = self.writer.finish(incomplete)?;
        tape_write_eof_mark(self.writer.writer_ref_mut())?;
        Ok(leof)
    }

    fn logical_end_of_media(&self) -> bool {
        self.writer.logical_end_of_media()
    }
}
