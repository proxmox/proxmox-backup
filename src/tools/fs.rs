//! File system helper utilities.

use std::borrow::{Borrow, BorrowMut};
use std::ops::{Deref, DerefMut};
use std::os::unix::io::{AsRawFd, RawFd};

use failure::*;
use nix::dir;
use nix::dir::Dir;
use regex::Regex;

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
) -> Result<impl Iterator<Item = Result<ReadDirEntry, Error>> + 'a, Error> {
    Ok(read_subdir(dirfd, path)?.filter_file_name_regex(regex))
}

/// Helper trait to provide a combinators for directory entry iterators.
pub trait FileIterOps<T, E>
where
    Self: Sized + Iterator<Item = Result<T, E>>,
    T: Borrow<dir::Entry>,
    E: Into<Error> + Send + Sync,
{
    /// Filter by file type. This is more convenient than using the `filter` method alone as this
    /// also includes error handling and handling of files without a type (via an error).
    fn filter_file_type(self, ty: dir::Type) -> FileTypeFilter<Self, T, E> {
        FileTypeFilter { inner: self, ty }
    }

    /// Filter by file name. Note that file names which aren't valid utf-8 will be treated as if
    /// they do not match the pattern.
    fn filter_file_name_regex<'a>(self, regex: &'a Regex) -> FileNameRegexFilter<'a, Self, T, E> {
        FileNameRegexFilter { inner: self, regex }
    }
}

impl<I, T, E> FileIterOps<T, E> for I
where
    I: Iterator<Item = Result<T, E>>,
    T: Borrow<dir::Entry>,
    E: Into<Error> + Send + Sync,
{
}

/// This filters files from its inner iterator by a file type. Files with no type produce an error.
pub struct FileTypeFilter<I, T, E>
where
    I: Iterator<Item = Result<T, E>>,
    T: Borrow<dir::Entry>,
    E: Into<Error> + Send + Sync,
{
    inner: I,
    ty: nix::dir::Type,
}

impl<I, T, E> Iterator for FileTypeFilter<I, T, E>
where
    I: Iterator<Item = Result<T, E>>,
    T: Borrow<dir::Entry>,
    E: Into<Error> + Send + Sync,
{
    type Item = Result<T, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let item = self.inner.next()?.map_err(|e| e.into());
            match item {
                Ok(ref entry) => match entry.borrow().file_type() {
                    Some(ty) => {
                        if ty == self.ty {
                            return Some(item);
                        } else {
                            continue;
                        }
                    }
                    None => return Some(Err(format_err!("unable to detect file type"))),
                },
                Err(_) => return Some(item),
            }
        }
    }
}

/// This filters files by name via a Regex. Files whose file name aren't valid utf-8 are skipped
/// silently.
pub struct FileNameRegexFilter<'a, I, T, E>
where
    I: Iterator<Item = Result<T, E>>,
    T: Borrow<dir::Entry>,
{
    inner: I,
    regex: &'a Regex,
}

impl<I, T, E> Iterator for FileNameRegexFilter<'_, I, T, E>
where
    I: Iterator<Item = Result<T, E>>,
    T: Borrow<dir::Entry>,
{
    type Item = Result<T, E>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let item = self.inner.next()?;
            match item {
                Ok(ref entry) => {
                    if let Ok(name) = entry.borrow().file_name().to_str() {
                        if self.regex.is_match(name) {
                            return Some(item);
                        }
                    }
                    // file did not match regex or isn't valid utf-8
                    continue;
                },
                Err(_) => return Some(item),
            }
        }
    }
}
