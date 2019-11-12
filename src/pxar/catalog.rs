//! Trait for file list catalog
//!
//! A file list catalog simply store a directory tree. Such catalogs
//! may be used as index to do a fast search for files.

use failure::*;
use std::ffi::CStr;

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
