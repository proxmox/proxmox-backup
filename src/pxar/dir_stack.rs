use std::ffi::{OsStr, OsString};
use std::os::unix::io::{AsRawFd, RawFd};
use std::path::PathBuf;

use failure::{format_err, Error};
use nix::errno::Errno;
use nix::fcntl::OFlag;
use nix::sys::stat::Mode;
use nix::NixPath;

use super::format_definition::{PxarAttributes, PxarEntry};

pub struct PxarDir {
    pub filename: OsString,
    pub entry: PxarEntry,
    pub attr: PxarAttributes,
    pub dir: Option<nix::dir::Dir>,
}

pub struct PxarDirStack {
    root: RawFd,
    data: Vec<PxarDir>,
}

impl PxarDir {
    pub fn new(filename: &OsStr, entry: PxarEntry, attr: PxarAttributes) -> Self {
        Self {
            filename: filename.to_os_string(),
            entry,
            attr,
            dir: None,
        }
    }

    fn create_dir(&self, parent: RawFd, create_new: bool) -> Result<nix::dir::Dir, nix::Error> {
        let res = self
            .filename
            .with_nix_path(|cstr| unsafe { libc::mkdirat(parent, cstr.as_ptr(), libc::S_IRWXU) })?;

        match Errno::result(res) {
            Ok(_) => {}
            Err(err) => {
                if err == nix::Error::Sys(nix::errno::Errno::EEXIST) {
                    if create_new {
                        return Err(err);
                    }
                } else {
                    return Err(err);
                }
            }
        }

        let dir = nix::dir::Dir::openat(
            parent,
            self.filename.as_os_str(),
            OFlag::O_DIRECTORY,
            Mode::empty(),
        )?;

        Ok(dir)
    }
}

impl PxarDirStack {
    pub fn new(parent: RawFd) -> Self {
        Self {
            root: parent,
            data: Vec::new(),
        }
    }

    pub fn push(&mut self, dir: PxarDir) {
        self.data.push(dir);
    }

    pub fn pop(&mut self) -> Option<PxarDir> {
        self.data.pop()
    }

    pub fn as_path_buf(&self) -> PathBuf {
        let path: PathBuf = self.data.iter().map(|d| d.filename.clone()).collect();
        path
    }

    pub fn last(&self) -> Option<&PxarDir> {
        self.data.last()
    }

    pub fn last_mut(&mut self) -> Option<&mut PxarDir> {
        self.data.last_mut()
    }

    pub fn last_dir_fd(&self) -> Option<RawFd> {
        let last_dir = self.data.last()?;
        match &last_dir.dir {
            Some(d) => Some(d.as_raw_fd()),
            None => None,
        }
    }

    pub fn create_all_dirs(&mut self, create_new: bool) -> Result<RawFd, Error> {
        let mut current_fd = self.root;
        for d in &mut self.data {
            match &d.dir {
                Some(dir) => current_fd = dir.as_raw_fd(),
                None => {
                    let dir = d
                        .create_dir(current_fd, create_new)
                        .map_err(|err| format_err!("create dir failed - {}", err))?;
                    current_fd = dir.as_raw_fd();
                    d.dir = Some(dir);
                }
            }
        }

        Ok(current_fd)
    }
}
