use failure::*;

use super::format_definition::*;

use std::io::Write;
use std::os::unix::io::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::io::RawFd;

// PATH_MAX on Linux is 4096 (NUL byte already included)
const PATH_MAX: usize = 4096;

pub struct CaTarEncoder<W: Write> {
    current_path: std::path::PathBuf, // used for error reporting
    writer: W,
    size: usize,
}

impl <W: Write> CaTarEncoder<W> {

    pub fn encode(mut path: std::path::PathBuf, dir: &mut nix::dir::Dir, writer: W) -> Result<(), Error> {
        let mut me = Self {
            current_path: path,
            writer: writer,
            size: 0,
        };

        // todo: use scandirat??

        me.encode_dir(dir)?;

        Ok(())
    }

    fn encode_dir(&mut self, dir: &mut nix::dir::Dir)  -> Result<(), Error> {

        println!("encode_dir: {:?}", self.current_path);

        let mut name_list = vec![];

        let rawfd = dir.as_raw_fd();

        let dir_stat = match nix::sys::stat::fstat(rawfd) {
            Ok(stat) => stat,
            Err(err) => bail!("fstat failed - {}", err),
        };

        for entry in dir.iter() {
            let entry = match entry {
                Ok(entry) => entry,
                Err(err) => bail!("readir failed - {}", err),
            };
            let filename = entry.file_name().to_owned();

            let name = filename.to_bytes_with_nul();
            let name_len = name.len();
            if name_len == 2 && name[0] == b'.' && name[1] == 0u8 { continue; }
            if name_len == 3 && name[0] == b'.' && name[1] == b'.' && name[2] == 0u8 { continue; }

            if let Ok(stat) = nix::sys::stat::fstatat(rawfd, filename.as_ref(), nix::fcntl::AtFlags::AT_SYMLINK_NOFOLLOW) {
                //println!("Found {:?}", filename);
                name_list.push((filename, stat));
            } else {
                bail!("fsstat failed");
            }
        }

        name_list.sort_unstable_by(|a, b| a.0.cmp(&b.0));

        for (filename, stat) in name_list {
            //println!("SORTED {:?}", filename);
            self.current_path.push(std::ffi::OsStr::from_bytes(filename.as_bytes()));

            if (stat.st_mode & libc::S_IFMT) == libc::S_IFDIR {

                let mut dir = nix::dir::Dir::openat(
                    rawfd, filename.as_ref(), nix::fcntl::OFlag::O_NOFOLLOW, nix::sys::stat::Mode::empty())?;

                self.encode_dir(&mut dir)?;
            } else if (stat.st_mode & libc::S_IFMT) == libc::S_IFREG {
                let filefd = nix::fcntl::openat(rawfd, filename.as_ref(), nix::fcntl::OFlag::O_NOFOLLOW, nix::sys::stat::Mode::empty())?;
                self.encode_file(filefd);
                nix::unistd::close(filefd);
            } else if (stat.st_mode & libc::S_IFMT) == libc::S_IFLNK {
                let mut buffer = [0u8; PATH_MAX];
                let target = nix::fcntl::readlinkat(rawfd, filename.as_ref(), &mut buffer)?;
                self.encode_symlink(&target)?;
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
}
