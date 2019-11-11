//! Trait for file list catalog
//!
//! A file list catalog simply store a directory tree. Such catalogs
//! may be used as index to do a fast search for files.

use failure::*;
use std::convert::TryFrom;
use std::ffi::CStr;
use std::fmt;

#[repr(u8)]
#[derive(Copy,Clone,PartialEq)]
pub enum CatalogEntryType {
    Directory = b'd',
    File = b'f',
    Symlink = b'l',
    Hardlink = b'h',
    BlockDevice = b'b',
    CharDevice = b'c',
    Fifo = b'p', // Fifo,Pipe
    Socket = b's',
}

impl TryFrom<u8> for CatalogEntryType {
    type Error=Error;

    fn try_from(value: u8) -> Result<Self, Error> {
        Ok(match value {
            b'd' => CatalogEntryType::Directory,
            b'f' => CatalogEntryType::File,
            b'l' => CatalogEntryType::Symlink,
            b'h' => CatalogEntryType::Hardlink,
            b'b' => CatalogEntryType::BlockDevice,
            b'c' => CatalogEntryType::CharDevice,
            b'p' => CatalogEntryType::Fifo,
            b's' => CatalogEntryType::Socket,
            _ => bail!("invalid CatalogEntryType value '{}'", char::from(value)),
        })
    }
}

impl fmt::Display for CatalogEntryType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", char::from(*self as u8))
    }
}

pub trait BackupCatalogWriter {
    fn start_directory(&mut self, name: &CStr) -> Result<(), Error>;
    fn end_directory(&mut self) -> Result<(), Error>;
    fn add_file(&mut self, name: &CStr, size: u64, mtime: u64) -> Result<(), Error>;
    fn add_symlink(&mut self, name: &CStr) -> Result<(), Error>;
    fn add_hardlink(&mut self, name: &CStr) -> Result<(), Error>;
    fn add_block_device(&mut self, name: &CStr) -> Result<(), Error>;
    fn add_char_device(&mut self, name: &CStr) -> Result<(), Error>;
    fn add_fifo(&mut self, name: &CStr) -> Result<(), Error>;
    fn add_socket(&mut self, name: &CStr) -> Result<(), Error>;
}

pub struct DummyCatalogWriter();

impl BackupCatalogWriter for DummyCatalogWriter {
    fn start_directory(&mut self, _name: &CStr) -> Result<(), Error> { Ok(()) }
    fn end_directory(&mut self) -> Result<(), Error> { Ok(()) }
    fn add_file(&mut self, _name: &CStr, _size: u64, _mtime: u64) -> Result<(), Error> { Ok(()) }
    fn add_symlink(&mut self, _name: &CStr) -> Result<(), Error> { Ok(()) }
    fn add_hardlink(&mut self, _name: &CStr) -> Result<(), Error> { Ok(()) }
    fn add_block_device(&mut self, _name: &CStr) -> Result<(), Error> { Ok(()) }
    fn add_char_device(&mut self, _name: &CStr) -> Result<(), Error> { Ok(()) }
    fn add_fifo(&mut self, _name: &CStr) -> Result<(), Error> { Ok(()) }
    fn add_socket(&mut self, _name: &CStr) -> Result<(), Error> { Ok(()) }
}
