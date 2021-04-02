use std::ffi::OsString;
use std::os::unix::io::{AsRawFd, RawFd};
use std::path::{Path, PathBuf};

use anyhow::{bail, format_err, Error};
use nix::dir::Dir;
use nix::fcntl::OFlag;
use nix::sys::stat::{mkdirat, Mode};

use proxmox::sys::error::SysError;
use proxmox::tools::fd::BorrowedFd;
use pxar::Metadata;

use crate::pxar::tools::{assert_single_path_component, perms_from_metadata};

pub struct PxarDir {
    file_name: OsString,
    metadata: Metadata,
    dir: Option<Dir>,
}

impl PxarDir {
    pub fn new(file_name: OsString, metadata: Metadata) -> Self {
        Self {
            file_name,
            metadata,
            dir: None,
        }
    }

    pub fn with_dir(dir: Dir, metadata: Metadata) -> Self {
        Self {
            file_name: OsString::from("."),
            metadata,
            dir: Some(dir),
        }
    }

    fn create_dir(
        &mut self,
        parent: RawFd,
        allow_existing_dirs: bool,
    ) -> Result<BorrowedFd, Error> {
        match mkdirat(
            parent,
            self.file_name.as_os_str(),
            perms_from_metadata(&self.metadata)?,
        ) {
            Ok(()) => (),
            Err(err) => {
                if !(allow_existing_dirs && err.already_exists()) {
                    return Err(err.into());
                }
            }
        }

        self.open_dir(parent)
    }

    fn open_dir(&mut self, parent: RawFd) -> Result<BorrowedFd, Error> {
        let dir = Dir::openat(
            parent,
            self.file_name.as_os_str(),
            OFlag::O_DIRECTORY,
            Mode::empty(),
        )?;

        let fd = BorrowedFd::new(&dir);
        self.dir = Some(dir);

        Ok(fd)
    }

    pub fn try_as_borrowed_fd(&self) -> Option<BorrowedFd> {
        self.dir.as_ref().map(BorrowedFd::new)
    }

    pub fn metadata(&self) -> &Metadata {
        &self.metadata
    }
}

pub struct PxarDirStack {
    dirs: Vec<PxarDir>,
    path: PathBuf,
    created: usize,
}

impl PxarDirStack {
    pub fn new(root: Dir, metadata: Metadata) -> Self {
        Self {
            dirs: vec![PxarDir::with_dir(root, metadata)],
            path: PathBuf::from("/"),
            created: 1, // the root directory exists
        }
    }

    pub fn is_empty(&self) -> bool {
        self.dirs.is_empty()
    }

    pub fn push(&mut self, file_name: OsString, metadata: Metadata) -> Result<(), Error> {
        assert_single_path_component(&file_name)?;
        self.path.push(&file_name);
        self.dirs.push(PxarDir::new(file_name, metadata));
        Ok(())
    }

    pub fn pop(&mut self) -> Result<Option<PxarDir>, Error> {
        let out = self.dirs.pop();
        if !self.path.pop() {
            if self.path.as_os_str() == "/" {
                // we just finished the root directory, make sure this can only happen once:
                self.path = PathBuf::new();
            } else {
                bail!("lost track of path");
            }
        }
        self.created = self.created.min(self.dirs.len());
        Ok(out)
    }

    pub fn last_dir_fd(&mut self, allow_existing_dirs: bool) -> Result<BorrowedFd, Error> {
        // should not be possible given the way we use it:
        assert!(!self.dirs.is_empty(), "PxarDirStack underrun");

        let dirs_len = self.dirs.len();
        let mut fd = self.dirs[self.created - 1]
            .try_as_borrowed_fd()
            .ok_or_else(|| format_err!("lost track of directory file descriptors"))?
            .as_raw_fd();

        while self.created < dirs_len {
            fd = self.dirs[self.created]
                .create_dir(fd, allow_existing_dirs)?
                .as_raw_fd();
            self.created += 1;
        }

        self.dirs[self.created - 1]
            .try_as_borrowed_fd()
            .ok_or_else(|| format_err!("lost track of directory file descriptors"))
    }

    pub fn create_last_dir(&mut self, allow_existing_dirs: bool) -> Result<(), Error> {
        let _: BorrowedFd = self.last_dir_fd(allow_existing_dirs)?;
        Ok(())
    }

    pub fn root_dir_fd(&self) -> Result<BorrowedFd, Error> {
        // should not be possible given the way we use it:
        assert!(!self.dirs.is_empty(), "PxarDirStack underrun");

        self.dirs[0]
            .try_as_borrowed_fd()
            .ok_or_else(|| format_err!("lost track of directory file descriptors"))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}
