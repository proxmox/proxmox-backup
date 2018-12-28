use failure::*;

use super::format_definition::*;

use std::io::Write;
use std::os::unix::io::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::io::RawFd;
use std::path::{Path, PathBuf};

use nix::fcntl::OFlag;
use nix::sys::stat::Mode;
use nix::errno::Errno;

pub struct CaTarEncoder<W: Write> {
    current_path: PathBuf, // used for error reporting
    writer: W,
    size: usize,
}

impl <W: Write> CaTarEncoder<W> {

    pub fn encode(path: PathBuf, dir: &mut nix::dir::Dir, writer: W) -> Result<(), Error> {
        let mut me = Self {
            current_path: path,
            writer: writer,
            size: 0,
        };

        // todo: use scandirat??

        me.encode_dir(dir)?;

        Ok(())
    }

    //fn report_vanished

    fn encode_dir(&mut self, dir: &mut nix::dir::Dir)  -> Result<(), Error> {

        println!("encode_dir: {:?}", self.current_path);

        let mut name_list = vec![];

        let rawfd = dir.as_raw_fd();

        let dir_stat = match nix::sys::stat::fstat(rawfd) {
            Ok(stat) => stat,
            Err(err) => bail!("fstat {:?} failed - {}", self.current_path, err),
        };

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

        for (filename, stat) in name_list {
            self.current_path.push(std::ffi::OsStr::from_bytes(filename.as_bytes()));

            if (stat.st_mode & libc::S_IFMT) == libc::S_IFDIR {
                match nix::dir::Dir::openat(rawfd, filename.as_ref(), OFlag::O_NOFOLLOW, Mode::empty()) {
                    Ok(mut dir) => self.encode_dir(&mut dir)?,
                    Err(nix::Error::Sys(Errno::ENOENT)) => self.report_vanished_file(&self.current_path)?,
                    Err(err) => bail!("open dir {:?} failed - {}", self.current_path, err),
                }
                
            } else if (stat.st_mode & libc::S_IFMT) == libc::S_IFREG {
                match nix::fcntl::openat(rawfd, filename.as_ref(), OFlag::O_NOFOLLOW, Mode::empty()) {
                    Ok(filefd) => {
                        let res = self.encode_file(filefd);
                        let _ = nix::unistd::close(filefd); // ignore close errors
                        res?;
                    }
                    Err(nix::Error::Sys(Errno::ENOENT)) => self.report_vanished_file(&self.current_path)?,
                    Err(err) => bail!("open file {:?} failed - {}", self.current_path, err),
                }
            } else if (stat.st_mode & libc::S_IFMT) == libc::S_IFLNK {
                let mut buffer = [0u8; libc::PATH_MAX as usize];
                match nix::fcntl::readlinkat(rawfd, filename.as_ref(), &mut buffer) {
                    Ok(target) => self.encode_symlink(&target)?,
                    Err(nix::Error::Sys(Errno::ENOENT)) => self.report_vanished_file(&self.current_path)?,
                    Err(err) => bail!("readlink {:?} failed - {}", self.current_path, err),
                }
            } else {
                bail!("unsupported file type (mode {:o} {:?})", stat.st_mode, self.current_path);
            }

            self.current_path.pop();
         }

        Ok(())
    }

    fn encode_file(&mut self, filefd: RawFd)  -> Result<(), Error> {

        println!("encode_file: {:?}", self.current_path);

        Ok(())
    }

    fn encode_symlink(&mut self, target: &std::ffi::OsStr)  -> Result<(), Error> {

        println!("encode_symlink: {:?} -> {:?}", self.current_path, target);

        Ok(())
    }

    // the report_XXX method may raise and error - depending on encoder configuration
    
    fn report_vanished_file(&self, path: &Path) -> Result<(), Error> {

        eprintln!("WARNING: detected vanished file {:?}", path);

        Ok(())
    }

}
