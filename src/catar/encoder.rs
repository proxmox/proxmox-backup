use failure::*;

use super::format_definition::*;

use std::io::Write;
use std::os::unix::io::AsRawFd;
use std::os::unix::ffi::OsStrExt;

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
            let mut filename = entry.file_name().to_owned();

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

            if (stat.st_mode & libc::S_IFDIR) != 0 {

                let mut dir = nix::dir::Dir::openat(
                    rawfd, filename.as_ref(), nix::fcntl::OFlag::O_NOFOLLOW, nix::sys::stat::Mode::empty())?;

                self.current_path.push(std::ffi::OsStr::from_bytes(filename.as_bytes()));
                self.encode_dir(&mut dir)?;
                self.current_path.pop();
            }
        }

        Ok(())
    }
}
