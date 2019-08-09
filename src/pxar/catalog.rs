//! File list Catalog
//!
//! A file list catalog simply store a directory tree. Such catalogs
//! may be used as index to do a fast search for files.

use failure::*;

use std::io::{Read, BufRead, Write, BufReader};
use std::fs::File;
use std::convert::TryFrom;

use std::ffi::{CStr, CString};

use chrono::offset::{TimeZone, Local};

use proxmox::tools::io::ReadExt;

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

pub struct SimpleCatalog {
    writer: std::fs::File,
    level: usize,
}

impl SimpleCatalog {
    pub fn new<P: AsRef<std::path::Path>>(path: P) -> Result<Self, Error> {
        let writer = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(path.as_ref())?;

        Ok(Self { writer, level: 0 })
    }
}

impl BackupCatalogWriter for SimpleCatalog {

    fn start_directory(&mut self, name: &CStr) -> Result<(), Error> {
        self.writer.write(&[CatalogEntryType::Directory as u8])?;
        self.writer.write(name.to_bytes_with_nul())?;
        self.writer.write(b"{")?;
        self.level += 1;
        Ok(())
    }

    fn end_directory(&mut self) -> Result<(), Error> {
        if self.level == 0 {
            bail!("got unexpected end_directory level 0");
        }
        self.writer.write(b"}")?;
        self.level -= 1;
        Ok(())
    }

    fn add_file(&mut self, name: &CStr, size: u64, mtime: u64) -> Result<(), Error> {
        self.writer.write(&[CatalogEntryType::File as u8])?;
        self.writer.write(&size.to_le_bytes())?;
        self.writer.write(&mtime.to_le_bytes())?;
        self.writer.write(name.to_bytes_with_nul())?;
        Ok(())
    }

    fn add_symlink(&mut self, name: &CStr) -> Result<(), Error> {
        self.writer.write(&[CatalogEntryType::Symlink as u8])?;
        self.writer.write(name.to_bytes_with_nul())?;
        Ok(())
    }

    fn add_hardlink(&mut self, name: &CStr) -> Result<(), Error> {
        self.writer.write(&[CatalogEntryType::Hardlink as u8])?;
        self.writer.write(name.to_bytes_with_nul())?;
        Ok(())
    }

    fn add_block_device(&mut self, name: &CStr) -> Result<(), Error> {
        self.writer.write(&[CatalogEntryType::BlockDevice as u8])?;
        self.writer.write(name.to_bytes_with_nul())?;
        Ok(())
    }

    fn add_char_device(&mut self, name: &CStr) -> Result<(), Error> {
        self.writer.write(&[CatalogEntryType::CharDevice as u8])?;
        self.writer.write(name.to_bytes_with_nul())?;
        Ok(())
    }

    fn add_fifo(&mut self, name: &CStr) -> Result<(), Error> {
        self.writer.write(&[CatalogEntryType::Fifo as u8])?;
        self.writer.write(name.to_bytes_with_nul())?;
        Ok(())
    }

    fn add_socket(&mut self, name: &CStr) -> Result<(), Error> {
        self.writer.write(&[CatalogEntryType::Socket as u8])?;
        self.writer.write(name.to_bytes_with_nul())?;
        Ok(())
    }
}

pub struct SimpleCatalogReader {
    reader: BufReader<File>,
    dir_stack: Vec<CString>,
}

impl SimpleCatalogReader {

    pub fn open<P: AsRef<std::path::Path>>(path: P) -> Result<Self, Error> {
        let file = std::fs::File::open(path)?;
        let reader = BufReader::new(file);
        let dir_stack = Vec::new();
        Ok(Self { reader, dir_stack })
    }

    fn read_filename(&mut self) ->  Result<std::ffi::CString, Error> {
        let mut filename = Vec::new();
        self.reader.read_until(0u8, &mut filename)?;
        if filename.len() > 0 && filename[filename.len()-1] == 0u8 {
            filename.pop();
        }
        if filename.len() == 0 {
            bail!("got zero length filename");
        }
        if filename.iter().find(|b| **b == b'/').is_some() {
            bail!("found invalid filename with slashes.");
        }
        Ok(unsafe { CString::from_vec_unchecked(filename) })
    }

    fn next_byte(&mut self) ->  Result<u8, std::io::Error> {
        let mut buf = [0u8; 1];
        self.reader.read_exact(&mut buf)?;
        Ok(buf[0])
    }

    fn expect_next(&mut self, expect: u8) -> Result<(), Error> {
        let next = self.next_byte()?;
        if next != expect {
            bail!("got unexpected byte ({} != {})", next, expect);
        }
        Ok(())
    }

    fn print_entry(&self, etype: CatalogEntryType, filename: &CStr, size: u64, mtime: u64)  -> Result<(), Error> {
        let mut out = Vec::new();

        write!(out, "{} ", char::from(etype as u8))?;

        for name in &self.dir_stack {
            out.extend(name.to_bytes());
            out.push(b'/');
        }

        out.extend(filename.to_bytes());

        let dt = Local.timestamp(mtime as i64, 0);

        if etype == CatalogEntryType::File {
            write!(out, " {} {}", size, dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, false))?;
        }

        write!(out, "\n")?;
        std::io::stdout().write_all(&out)?;

        Ok(())
    }

    fn parse_entries(&mut self) ->  Result<(), Error> {

        loop {
            let etype = match self.next_byte() {
                Ok(v) => v,
                Err(err) => {
                    if err.kind() == std::io::ErrorKind::UnexpectedEof {
                        if self.dir_stack.len() == 0 {
                            break;
                        }
                    }
                    return Err(err.into());
                }
            };
            if etype == b'}' {
                if self.dir_stack.pop().is_none() {
                    bail!("got unexpected '}'");
                }
                break;
            }

            let etype = CatalogEntryType::try_from(etype)?;
            match etype {
                CatalogEntryType::Directory => {
                    let filename = self.read_filename()?;
                    self.print_entry(etype.into(), &filename, 0, 0)?;
                    self.dir_stack.push(filename);
                    self.expect_next(b'{')?;
                    self.parse_entries()?;
                }
                CatalogEntryType::File => {
                    let size = unsafe { self.reader.read_le_value::<u64>()? };
                    let mtime = unsafe { self.reader.read_le_value::<u64>()? };
                    let filename = self.read_filename()?;
                    self.print_entry(etype.into(), &filename, size, mtime)?;
                }
                CatalogEntryType::Symlink |
                CatalogEntryType::Hardlink |
                CatalogEntryType::Fifo |
                CatalogEntryType::Socket |
                CatalogEntryType::BlockDevice |
                CatalogEntryType::CharDevice => {
                    let filename = self.read_filename()?;
                    self.print_entry(etype.into(), &filename, 0, 0)?;
                }
            }
        }
        Ok(())
    }

    pub fn dump(&mut self) -> Result<(), Error> {
        self.parse_entries()?;
        Ok(())
    }
}
