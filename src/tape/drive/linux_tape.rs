use std::fs::{OpenOptions, File};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;
use std::convert::TryFrom;

use anyhow::{bail, format_err, Error};
use nix::fcntl::{fcntl, FcntlArg, OFlag};

use proxmox::sys::error::SysResult;

use crate::{
    api2::types::{
        TapeDensity,
        MamAttribute,
    },
    tape::{
        TapeRead,
        TapeWrite,
        read_mam_attributes,
        drive::{
            LinuxTapeDrive,
            TapeDriver,
            linux_mtio::*,
        },
        file_formats::{
            PROXMOX_TAPE_BLOCK_SIZE,
            MediaSetLabel,
            MediaContentHeader,
            PROXMOX_BACKUP_MEDIA_SET_LABEL_MAGIC_1_0,
        },
        helpers::{
            BlockedReader,
            BlockedWriter,
        },
    }
};

#[derive(Debug)]
pub struct LinuxDriveStatus {
    pub blocksize: u32,
    pub density: TapeDensity,
    pub status: GMTStatusFlags,
    pub file_number: i32,
    pub block_number: i32,
}

impl LinuxDriveStatus {
    pub fn tape_is_ready(&self) -> bool {
        self.status.contains(GMTStatusFlags::ONLINE) &&
            !self.status.contains(GMTStatusFlags::DRIVE_OPEN)
    }
}

impl LinuxTapeDrive {

    /// This needs to lock the drive
    pub fn open(&self) -> Result<LinuxTapeHandle, Error> {

        let file = open_linux_tape_device(&self.path)?;

        let handle = LinuxTapeHandle { drive_name: self.name.clone(), file };

        let drive_status = handle.get_drive_status()?;
        println!("drive status: {:?}", drive_status);

        if !drive_status.tape_is_ready() {
            bail!("tape not ready (no tape loaded)");
        }

        if drive_status.blocksize == 0 {
            eprintln!("device is variable block size");
        } else {
            if drive_status.blocksize != PROXMOX_TAPE_BLOCK_SIZE as u32 {
                eprintln!("device is in fixed block size mode with wrong size ({} bytes)", drive_status.blocksize);
                eprintln!("trying to set variable block size mode...");
                if handle.set_block_size(0).is_err() {
                     bail!("set variable block size mod failed - device uses wrong blocksize.");
                 }
            } else {
                 eprintln!("device is in fixed block size mode ({} bytes)", drive_status.blocksize);
            }
        }

        // Only root can seth driver options, so we cannot
        // handle.set_default_options()?;

        Ok(handle)
    }
}

pub struct LinuxTapeHandle {
    drive_name: String,
    file: File,
    //_lock: File,
}

impl LinuxTapeHandle {

    /// Return the drive name (useful for log and debug)
    pub fn dive_name(&self) -> &str {
        &self.drive_name
    }

    /// Set all options we need/want
    pub fn set_default_options(&self) -> Result<(), Error> {

        let mut opts = SetDrvBufferOptions::empty();

        // fixme: ? man st(4) claims we need to clear this for reliable multivolume
        opts.set(SetDrvBufferOptions::MT_ST_BUFFER_WRITES, true);

        // fixme: ?man st(4) claims we need to clear this for reliable multivolume
        opts.set(SetDrvBufferOptions::MT_ST_ASYNC_WRITES, true);

        opts.set(SetDrvBufferOptions::MT_ST_READ_AHEAD, true);

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

    /// This flushes the driver's buffer as a side effect. Should be
    /// used before reading status with MTIOCGET.
    fn mtnop(&self) -> Result<(), Error> {

        let cmd = mtop { mt_op: MTCmd::MTNOP, mt_count: 1, };

        unsafe {
            mtioctop(self.file.as_raw_fd(), &cmd)
        }.map_err(|err| format_err!("MTNOP failed - {}", err))?;

        Ok(())
    }

    fn forward_space_count_files(&mut self, count: i32) -> Result<(), Error> {

        let cmd = mtop { mt_op: MTCmd::MTFSF, mt_count: count, };

        unsafe {
            mtioctop(self.file.as_raw_fd(), &cmd)
        }.map_err(|err| format_err!("tape fsf {} failed - {}", count, err))?;

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

    /// Get Tape configuration with MTIOCGET ioctl
    pub fn get_drive_status(&self) -> Result<LinuxDriveStatus, Error> {

        self.mtnop()?;

        let mut status = mtget::default();

        if let Err(err) = unsafe { mtiocget(self.file.as_raw_fd(), &mut status) } {
            bail!("MTIOCGET failed - {}", err);
        }

        println!("{:?}", status);

        let gmt = GMTStatusFlags::from_bits_truncate(status.mt_gstat);

        let blocksize;

        if status.mt_type == MT_TYPE_ISSCSI1 || status.mt_type == MT_TYPE_ISSCSI2 {
            blocksize = ((status.mt_dsreg & MT_ST_BLKSIZE_MASK) >> MT_ST_BLKSIZE_SHIFT) as u32;
        } else {
            bail!("got unsupported tape type {}", status.mt_type);
        }

        let density = ((status.mt_dsreg & MT_ST_DENSITY_MASK) >> MT_ST_DENSITY_SHIFT) as u8;

        let density = TapeDensity::try_from(density)?;

        Ok(LinuxDriveStatus {
            blocksize,
            density,
            status: gmt,
            file_number: status.mt_fileno,
            block_number: status.mt_blkno,
        })
    }

    /// Read Cartridge Memory (MAM Attributes)
    pub fn cartridge_memory(&mut self) -> Result<Vec<MamAttribute>, Error> {
        read_mam_attributes(&mut self.file)
    }
}


impl TapeDriver for LinuxTapeHandle {

    fn sync(&mut self) -> Result<(), Error> {

        println!("SYNC/FLUSH TAPE");
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

    fn write_media_set_label(&mut self, media_set_label: &MediaSetLabel) -> Result<(), Error> {

        let file_number = self.current_file_number()?;
        if file_number != 1 {
            self.rewind()?;
            self.forward_space_count_files(1)?; // skip label
        }

        let file_number = self.current_file_number()?;
        if file_number != 1 {
            bail!("write_media_set_label failed - got wrong file number ({} != 1)", file_number);
        }

        let mut handle = TapeWriterHandle {
            writer: BlockedWriter::new(&mut self.file),
        };
        let raw = serde_json::to_string_pretty(&serde_json::to_value(media_set_label)?)?;

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
}

/// Write a single EOF mark without flushing buffers
fn tape_write_eof_mark(file: &File) -> Result<(), std::io::Error> {

    println!("WRITE EOF MARK");
    let cmd = mtop { mt_op: MTCmd::MTWEOFI, mt_count: 1 };

    unsafe {
        mtioctop(file.as_raw_fd(), &cmd)
    }.map_err(|err| proxmox::io_format_err!("MTWEOFI failed - {}", err))?;

    Ok(())
}

fn tape_is_linux_tape_device(file: &File) -> bool {

    let devnum = match nix::sys::stat::fstat(file.as_raw_fd()) {
        Ok(stat) => stat.st_rdev,
        _ => return false,
    };

    let major = unsafe { libc::major(devnum) };
    let minor = unsafe { libc::minor(devnum) };

    if major != 9 { return false; } // The st driver uses major device number 9
    if (minor & 128) == 0 {
        eprintln!("Detected rewinding tape. Please use non-rewinding tape devices (/dev/nstX).");
        return false;
    }

    true
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

    if !tape_is_linux_tape_device(&file) {
        bail!("file {:?} is not a linux tape device", path);
    }

    Ok(file)
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
        println!("FINISH TAPE HANDLE");
        let leof = self.writer.finish(incomplete)?;
        tape_write_eof_mark(self.writer.writer_ref_mut())?;
        Ok(leof)
    }

    fn logical_end_of_media(&self) -> bool {
        self.writer.logical_end_of_media()
    }
}
