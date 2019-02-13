//! File system helper utilities.

use std::borrow::{Borrow, BorrowMut};
use std::ops::{Deref, DerefMut};
use std::os::unix::io::{AsRawFd, RawFd};

use failure::Error;
use nix::dir;
use nix::dir::Dir;

use crate::tools::borrow::Tied;

/// This wraps nix::dir::Entry with the parent directory's file descriptor.
pub struct ReadDirEntry {
    entry: dir::Entry,
    parent_fd: RawFd,
}

impl Into<dir::Entry> for ReadDirEntry {
    fn into(self) -> dir::Entry {
        self.entry
    }
}

impl Deref for ReadDirEntry {
    type Target = dir::Entry;

    fn deref(&self) -> &Self::Target {
        &self.entry
    }
}

impl DerefMut for ReadDirEntry {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.entry
    }
}

impl AsRef<dir::Entry> for ReadDirEntry {
    fn as_ref(&self) -> &dir::Entry {
        &self.entry
    }
}

impl AsMut<dir::Entry> for ReadDirEntry {
    fn as_mut(&mut self) -> &mut dir::Entry {
        &mut self.entry
    }
}

impl Borrow<dir::Entry> for ReadDirEntry {
    fn borrow(&self) -> &dir::Entry {
        &self.entry
    }
}

impl BorrowMut<dir::Entry> for ReadDirEntry {
    fn borrow_mut(&mut self) -> &mut dir::Entry {
        &mut self.entry
    }
}

impl ReadDirEntry {
    #[inline]
    pub fn parent_fd(&self) -> RawFd {
        self.parent_fd
    }
}

// Since Tied<T, U> implements Deref to U, a Tied<Dir, Iterator> already implements Iterator.
// This is simply a wrapper with a shorter type name mapping nix::Error to failure::Error.
/// Wrapper over a pair of `nix::dir::Dir` and `nix::dir::Iter`, returned by `read_subdir()`.
pub struct ReadDir {
    iter: Tied<Dir, Iterator<Item = nix::Result<dir::Entry>>>,
    dir_fd: RawFd,
}

impl Iterator for ReadDir {
    type Item = Result<ReadDirEntry, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        self.iter.next().map(|res| {
            res.map(|entry| ReadDirEntry { entry, parent_fd: self.dir_fd })
                .map_err(|e| Error::from(e))
        })
    }
}

/// Create an iterator over sub directory entries.
/// This uses `openat` on `dirfd`, so `path` can be relative to that or an absolute path.
pub fn read_subdir<P: ?Sized + nix::NixPath>(dirfd: RawFd, path: &P) -> Result<ReadDir, Error> {
    use nix::fcntl::OFlag;
    use nix::sys::stat::Mode;

    let dir = Dir::openat(dirfd, path, OFlag::O_RDONLY, Mode::empty())?;
    let fd = dir.as_raw_fd();
    let iter = Tied::new(dir, |dir| {
        Box::new(unsafe { (*dir).iter() }) as Box<Iterator<Item = nix::Result<dir::Entry>>>
    });
    Ok(ReadDir { iter, dir_fd: fd })
}

/// Scan through a directory with a regular expression. This is simply a shortcut filtering the
/// results of `read_subdir`. Non-UTF8 comaptible file names are silently ignored.
pub fn scan_subdir<'a, P: ?Sized + nix::NixPath>(
    dirfd: RawFd,
    path: &P,
    regex: &'a regex::Regex,
) -> Result<impl Iterator<Item = Result<nix::dir::Entry, Error>> + 'a, Error> {
    Ok(read_subdir(dirfd, path)?.filter_map(move |entry| {
        match entry {
            Ok(entry) => match entry.file_name().to_str() {
                Ok(name) => {
                    if regex.is_match(name) {
                        Some(Ok(entry))
                    } else {
                        None // Skip values not matching the regex
                    }
                }
                // Skip values which aren't valid utf-8
                Err(_) => None,
            },
            // Propagate errors through
            Err(e) => Some(Err(Error::from(e))),
        }
    }))
}
