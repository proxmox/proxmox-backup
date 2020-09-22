//! Trait for file list catalog
//!
//! A file list catalog simply store a directory tree. Such catalogs
//! may be used as index to do a fast search for files.

use anyhow::{Error};
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
