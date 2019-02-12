//! File system helper utilities.

use std::os::unix::io::RawFd;

use failure::Error;
use nix::dir;
use nix::dir::Dir;

use crate::tools::borrow::Tied;

// Since Tied<T, U> implements Deref to U, a Tied<Dir, Iterator> already implements Iterator.
// This is simply a wrapper with a shorter type name mapping nix::Error to failure::Error.
/// Wrapper over a pair of `nix::dir::Dir` and `nix::dir::Iter`, returned by `read_subdir()`.
pub struct ReadDir {
    iter: Tied<Dir, Iterator<Item = nix::Result<dir::Entry>>>,
}

impl Iterator for ReadDir {
    type Item = Result<dir::Entry, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        self.iter.next().map(|res| res.map_err(|e| Error::from(e)))
    }
}

/// Create an iterator over sub directory entries.
/// This uses `openat` on `dirfd`, so `path` can be relative to that or an absolute path.
pub fn read_subdir<P: ?Sized + nix::NixPath>(dirfd: RawFd, path: &P) -> Result<ReadDir, Error> {
    use nix::fcntl::OFlag;
    use nix::sys::stat::Mode;

    let dir = Dir::openat(dirfd, path, OFlag::O_RDONLY, Mode::empty())?;
    let iter = Tied::new(dir, |dir| {
        Box::new(unsafe { (*dir).iter() }) as Box<Iterator<Item = nix::Result<dir::Entry>>>
    });
    Ok(ReadDir { iter })
}
