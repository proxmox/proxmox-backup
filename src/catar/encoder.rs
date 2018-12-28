use failure::*;

use super::format_definition::*;

use std::io::Write;
use std::os::unix::io::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::io::RawFd;
use std::path::{Path, PathBuf};

use std::ffi::{CStr, CString};

use nix::NixPath;
use nix::fcntl::OFlag;
use nix::sys::stat::Mode;
use nix::errno::Errno;
use nix::sys::stat::FileStat;

const FILE_COPY_BUFFER_SIZE: usize = 1024*1024;

pub struct CaTarEncoder<W: Write> {
    current_path: PathBuf, // used for error reporting
    writer: W,
    writer_pos: usize,
    size: usize,
    file_copy_buffer: Vec<u8>,
}

impl <W: Write> CaTarEncoder<W> {

    pub fn encode(path: PathBuf, dir: &mut nix::dir::Dir, writer: W) -> Result<(), Error> {

        let mut file_copy_buffer = Vec::with_capacity(FILE_COPY_BUFFER_SIZE);
        unsafe { file_copy_buffer.set_len(FILE_COPY_BUFFER_SIZE); }

        let mut me = Self {
            current_path: path,
            writer: writer,
            writer_pos: 0,
            size: 0,
            file_copy_buffer,
        };


        // todo: use scandirat??

        let name = CString::new(".")?;
        me.encode_dir(dir, &name)?;

        Ok(())
    }

    fn write(&mut self,  buf: &[u8]) -> Result<(), Error> {
        self.writer.write(buf)?;
        self.writer_pos += buf.len();
        Ok(())
    }

    fn flush_copy_buffer(&mut self, size: usize) -> Result<(), Error> {
        self.writer.write(&self.file_copy_buffer)?;
        self.writer_pos += size;
        Ok(())
    }

    fn write_header(&mut self, htype: u64, size: u64) -> Result<(), Error> {

        let mut buffer = [0u8; std::mem::size_of::<CaFormatHeader>()];
        let mut header = crate::tools::map_struct_mut::<CaFormatHeader>(&mut buffer)?;
        header.size = u64::to_le((std::mem::size_of::<CaFormatHeader>() as u64) + size);
        header.htype = htype;

        self.write(&buffer)?;

        Ok(())
    }

    fn write_filename(&mut self, name: &CStr) -> Result<(), Error> {

        let buffer = name.to_bytes_with_nul();
        self.write_header(CA_FORMAT_FILENAME, buffer.len() as u64)?;
        self.write(buffer)?;

        Ok(())
    }

    fn write_entry(&mut self, stat: &FileStat) -> Result<(), Error> {

        let mut buffer = [0u8; std::mem::size_of::<CaFormatHeader>() + std::mem::size_of::<CaFormatEntry>()];
        let mut header = crate::tools::map_struct_mut::<CaFormatHeader>(&mut buffer)?;
        header.size = u64::to_le((std::mem::size_of::<CaFormatHeader>() + std::mem::size_of::<CaFormatEntry>()) as u64);
        header.htype = CA_FORMAT_ENTRY;

        let mut entry = crate::tools::map_struct_mut::<CaFormatEntry>(&mut buffer[std::mem::size_of::<CaFormatHeader>()..])?;

        entry.feature_flags = 0; // fixme ??

        if (stat.st_mode & libc::S_IFMT) == libc::S_IFLNK {
            entry.mode = u64::to_le((libc::S_IFLNK | 0o777) as u64);
        } else {
            let mode = stat.st_mode & (libc::S_IFMT | 0o7777);
            entry.mode = u64::to_le(mode as u64);
        }

        entry.flags = 0; // todo: CHATTR, FAT_ATTRS, subvolume?

        entry.uid = u64::to_le(stat.st_uid as u64);
        entry.gid = u64::to_le(stat.st_gid as u64);

        let mtime = stat.st_mtime * 1_000_000_000 + stat.st_mtime_nsec;
        if mtime > 0 { entry.mtime = mtime as u64 };

        self.write(&buffer)?;

        Ok(())
    }

    fn encode_dir(&mut self, dir: &mut nix::dir::Dir, name: &CStr)  -> Result<(), Error> {

        println!("encode_dir: {:?} start {}", self.current_path, self.writer_pos);

        let mut name_list = vec![];

        let rawfd = dir.as_raw_fd();

        let dir_stat = match nix::sys::stat::fstat(rawfd) {
            Ok(stat) => stat,
            Err(err) => bail!("fstat {:?} failed - {}", self.current_path, err),
        };

        if (dir_stat.st_mode & libc::S_IFMT) != libc::S_IFDIR {
            bail!("got unexpected file type {:?} (not a directory)", self.current_path);
        }

        self.write_entry(&dir_stat)?;

        self.write_filename(name)?;

        for entry in dir.iter() {
            let entry = match entry {
                Ok(entry) => entry,
                Err(err) => bail!("readir {:?} failed - {}", self.current_path, err),
            };
            let filename = entry.file_name().to_owned();

            let name = filename.to_bytes_with_nul();
            let name_len = name.len();
            if name_len == 2 && name[0] == b'.' && name[1] == 0u8 { continue; }
            if name_len == 3 && name[0] == b'.' && name[1] == b'.' && name[2] == 0u8 { continue; }

            match nix::sys::stat::fstatat(rawfd, filename.as_ref(), nix::fcntl::AtFlags::AT_SYMLINK_NOFOLLOW) {
                Ok(stat) => {
                    name_list.push((filename, stat));
                }
                Err(nix::Error::Sys(Errno::ENOENT)) => self.report_vanished_file(&self.current_path)?,
                Err(err) => bail!("fstat {:?} failed - {}", self.current_path, err),
            }
        }

        name_list.sort_unstable_by(|a, b| a.0.cmp(&b.0));

        for (filename, stat) in &name_list {
            self.current_path.push(std::ffi::OsStr::from_bytes(filename.as_bytes()));

            if (stat.st_mode & libc::S_IFMT) == libc::S_IFDIR {
                match nix::dir::Dir::openat(rawfd, filename.as_ref(), OFlag::O_NOFOLLOW, Mode::empty()) {
                    Ok(mut dir) => self.encode_dir(&mut dir, &filename)?,
                    Err(nix::Error::Sys(Errno::ENOENT)) => self.report_vanished_file(&self.current_path)?,
                    Err(err) => bail!("open dir {:?} failed - {}", self.current_path, err),
                }

            } else if (stat.st_mode & libc::S_IFMT) == libc::S_IFREG {
                match nix::fcntl::openat(rawfd, filename.as_ref(), OFlag::O_NOFOLLOW, Mode::empty()) {
                    Ok(filefd) => {
                        let res = self.encode_file(filefd, &filename);
                        let _ = nix::unistd::close(filefd); // ignore close errors
                        res?;
                    }
                    Err(nix::Error::Sys(Errno::ENOENT)) => self.report_vanished_file(&self.current_path)?,
                    Err(err) => bail!("open file {:?} failed - {}", self.current_path, err),
                }
            } else if (stat.st_mode & libc::S_IFMT) == libc::S_IFLNK {
                let mut buffer = [0u8; libc::PATH_MAX as usize];

                let res = filename.with_nix_path(|cstr| {
                    unsafe { libc::readlink(cstr.as_ptr(), buffer.as_mut_ptr() as *mut libc::c_char, buffer.len()) }
                })?;

                match Errno::result(res) {
                    Ok(len) => self.encode_symlink(&buffer[..(len as usize)], &stat, &filename)?,
                    Err(nix::Error::Sys(Errno::ENOENT)) => self.report_vanished_file(&self.current_path)?,
                    Err(err) => bail!("readlink {:?} failed - {}", self.current_path, err),
                }
            } else {
                bail!("unsupported file type (mode {:o} {:?})", stat.st_mode, self.current_path);
            }

            self.current_path.pop();
        }

        let entry_count = name_list.len();

        println!("encode_dir: {:?} end {}", self.current_path, self.writer_pos);

        Ok(())
    }

    fn encode_file(&mut self, filefd: RawFd, name: &CStr)  -> Result<(), Error> {

        println!("encode_file: {:?}", self.current_path);

        let stat = match nix::sys::stat::fstat(filefd) {
            Ok(stat) => stat,
            Err(err) => bail!("fstat {:?} failed - {}", self.current_path, err),
        };

        if (stat.st_mode & libc::S_IFMT) != libc::S_IFREG {
            bail!("got unexpected file type {:?} (not a regular file)", self.current_path);
        }

        self.write_entry(&stat)?;

        self.write_filename(name)?;

        let size = stat.st_size as u64;

        self.write_header(CA_FORMAT_PAYLOAD, size)?;

        let mut pos: u64 = 0;
        loop {
            let n = match nix::unistd::read(filefd, &mut self.file_copy_buffer) {
                Ok(n) => n,
                Err(nix::Error::Sys(Errno::EINTR)) => continue /* try again */,
                Err(err) =>  bail!("read {:?} failed - {}", self.current_path, err),
            };
            if n == 0 { // EOF
                if pos != size {
                    // Note:: casync format cannot handle that
                    bail!("detected shrinked file {:?} ({} < {})", self.current_path, pos, size);
                }
                break;
            }

            let mut next = pos + (n as u64);

            if next > size { next = size; }

            let count = (next - pos) as usize;

            self.flush_copy_buffer(count)?;

            pos += next;

            if pos >= size { break; }
        }

        Ok(())
    }

    fn encode_symlink(&mut self, target: &[u8], stat: &FileStat, name: &CStr)  -> Result<(), Error> {

        println!("encode_symlink: {:?} -> {:?}", self.current_path, target);

        self.write_entry(stat)?;

        self.write_filename(name)?;

        self.write_header(CA_FORMAT_SYMLINK, target.len() as u64)?;
        self.write(target)?;

        Ok(())
    }

    // the report_XXX method may raise and error - depending on encoder configuration

    fn report_vanished_file(&self, path: &Path) -> Result<(), Error> {

        eprintln!("WARNING: detected vanished file {:?}", path);

        Ok(())
    }

}
