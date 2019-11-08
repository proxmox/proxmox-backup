use failure::*;
use std::ffi::{CStr, CString};
use std::os::unix::ffi::OsStringExt;
use std::convert::TryInto;
use std::io::{Read, Write, Seek, SeekFrom};
use std::convert::TryFrom;

use chrono::offset::{TimeZone, Local};

use proxmox::tools::io::ReadExt;

use crate::pxar::catalog::{BackupCatalogWriter, CatalogEntryType};

enum DirEntry {
    Directory { name: Vec<u8>, start: u64 },
    File { name: Vec<u8>, size: u64, mtime: u64 },
    Symlink { name: Vec<u8> },
    Hardlink { name: Vec<u8> },
    BlockDevice { name: Vec<u8> },
    CharDevice { name: Vec<u8> },
    Fifo { name: Vec<u8> },
    Socket { name: Vec<u8> },
}

struct DirInfo {
    name: CString,
    entries: Vec<DirEntry>,
}

impl DirInfo {

    fn new(name: CString) -> Self {
        DirInfo { name, entries: Vec::new() }
    }

    fn new_rootdir() -> Self {
        DirInfo::new(CString::new(b"/".to_vec()).unwrap())
    }

    fn encode_entry(data: &mut Vec<u8>, entry: &DirEntry, pos: u64) {
        match entry {
            DirEntry::Directory { name, start } => {
                data.push(CatalogEntryType::Directory as u8);
                data.extend_from_slice(&(name.len() as u32).to_le_bytes());
                data.extend_from_slice(name);
                data.extend_from_slice(&(pos-start).to_le_bytes());
            }
            DirEntry::File { name, size, mtime } => {
                data.push(CatalogEntryType::File as u8);
                data.extend_from_slice(&(name.len() as u32).to_le_bytes());
                data.extend_from_slice(name);
                data.extend_from_slice(&size.to_le_bytes());
                data.extend_from_slice(&mtime.to_le_bytes());
            }
            DirEntry::Symlink { name } => {
                data.push(CatalogEntryType::Symlink as u8);
                data.extend_from_slice(&(name.len() as u32).to_le_bytes());
                data.extend_from_slice(name);
            }
            DirEntry::Hardlink { name } => {
                data.push(CatalogEntryType::Hardlink as u8);
                data.extend_from_slice(&(name.len() as u32).to_le_bytes());
                data.extend_from_slice(name);
            }
            DirEntry::BlockDevice { name } => {
                data.push(CatalogEntryType::BlockDevice as u8);
                data.extend_from_slice(&(name.len() as u32).to_le_bytes());
                data.extend_from_slice(name);
            }
             DirEntry::CharDevice { name } => {
                data.push(CatalogEntryType::CharDevice as u8);
                data.extend_from_slice(&(name.len() as u32).to_le_bytes());
                data.extend_from_slice(name);
            }
            DirEntry::Fifo { name } => {
                data.push(CatalogEntryType::Fifo as u8);
                data.extend_from_slice(&(name.len() as u32).to_le_bytes());
                data.extend_from_slice(name);
            }
            DirEntry::Socket { name } => {
                data.push(CatalogEntryType::Socket as u8);
                data.extend_from_slice(&(name.len() as u32).to_le_bytes());
                data.extend_from_slice(name);
            }
        }
    }

    fn encode(self, start: u64) -> Result<(CString, Vec<u8>), Error> {
        let mut table = Vec::new();
        let count: u32 = self.entries.len().try_into()?;
        for entry in self.entries {
            Self::encode_entry(&mut table, &entry, start);
        }

        let data = Vec::new();
        let mut writer = std::io::Cursor::new(data);
        let size: u32 = (4 + 4 + table.len()).try_into()?;
        writer.write_all(&size.to_le_bytes())?;
        writer.write_all(&count.to_le_bytes())?;
        writer.write_all(&table)?;
        Ok((self.name, writer.into_inner()))
    }
}

pub struct CatalogWriter<W> {
    writer: W,
    dirstack: Vec<DirInfo>,
    pos: u64,
}

impl <W: Write> CatalogWriter<W> {

    pub fn new(writer: W) -> Result<Self, Error> {
        Ok(Self { writer, dirstack: vec![ DirInfo::new_rootdir() ], pos: 0 })
    }

    pub fn finish(&mut self) -> Result<(), Error> {
        if self.dirstack.len() != 1 {
            bail!("unable to finish catalog at level {}", self.dirstack.len());
        }

        let dir = self.dirstack.pop().unwrap();

        let start = self.pos;
        let (_, data) = dir.encode(start)?;
        self.write_all(&data)?;

        self.write_all(&start.to_le_bytes())?;

        self.writer.flush()?;

        Ok(())
    }
}

impl <W: Write> BackupCatalogWriter for CatalogWriter<W> {

    fn start_directory(&mut self, name: &CStr) -> Result<(), Error> {
        let new = DirInfo::new(name.to_owned());
        self.dirstack.push(new);
        Ok(())
    }

    fn end_directory(&mut self) -> Result<(), Error> {
        let (start, name) = match self.dirstack.pop() {
            Some(dir) => {
                let start = self.pos;
                let (name, data) = dir.encode(start)?;
                self.write_all(&data)?;
                (start, name)
            }
            None => {
                bail!("got unexpected end_directory level 0");
            }
        };

        let current = self.dirstack.last_mut().ok_or_else(|| format_err!("outside root"))?;
        let name = name.to_bytes().to_vec();
        current.entries.push(DirEntry::Directory { name, start });

        Ok(())
    }

    fn add_file(&mut self, name: &CStr, size: u64, mtime: u64) -> Result<(), Error> {
        let dir = self.dirstack.last_mut().ok_or_else(|| format_err!("outside root"))?;
        let name = name.to_bytes().to_vec();
        dir.entries.push(DirEntry::File { name, size, mtime });
        Ok(())
    }

    fn add_symlink(&mut self, name: &CStr) -> Result<(), Error> {
        let dir = self.dirstack.last_mut().ok_or_else(|| format_err!("outside root"))?;
        let name = name.to_bytes().to_vec();
        dir.entries.push(DirEntry::Symlink { name });
        Ok(())
    }

    fn add_hardlink(&mut self, name: &CStr) -> Result<(), Error> {
        let dir = self.dirstack.last_mut().ok_or_else(|| format_err!("outside root"))?;
        let name = name.to_bytes().to_vec();
        dir.entries.push(DirEntry::Hardlink { name });
        Ok(())
    }

    fn add_block_device(&mut self, name: &CStr) -> Result<(), Error> {
        let dir = self.dirstack.last_mut().ok_or_else(|| format_err!("outside root"))?;
        let name = name.to_bytes().to_vec();
        dir.entries.push(DirEntry::BlockDevice { name });
         Ok(())
    }

    fn add_char_device(&mut self, name: &CStr) -> Result<(), Error> {
        let dir = self.dirstack.last_mut().ok_or_else(|| format_err!("outside root"))?;
        let name = name.to_bytes().to_vec();
        dir.entries.push(DirEntry::CharDevice { name });
        Ok(())
    }

    fn add_fifo(&mut self, name: &CStr) -> Result<(), Error> {
        let dir = self.dirstack.last_mut().ok_or_else(|| format_err!("outside root"))?;
        let name = name.to_bytes().to_vec();
        dir.entries.push(DirEntry::Fifo { name });
        Ok(())
    }

    fn add_socket(&mut self, name: &CStr) -> Result<(), Error> {
        let dir = self.dirstack.last_mut().ok_or_else(|| format_err!("outside root"))?;
        let name = name.to_bytes().to_vec();
        dir.entries.push(DirEntry::Socket { name });
        Ok(())
    }
}

impl<W: Write> CatalogWriter<W> {
    fn write_all(&mut self, data: &[u8]) -> Result<(), Error> {
        self.writer.write_all(data)?;
        self.pos += u64::try_from(data.len())?;
        Ok(())
    }
}

// fixme: move to somehere else?
/// Implement Write to tokio mpsc channel Sender
pub struct SenderWriter(tokio::sync::mpsc::Sender<Result<Vec<u8>, Error>>);

impl SenderWriter {
    pub fn new(sender: tokio::sync::mpsc::Sender<Result<Vec<u8>, Error>>) -> Self {
        Self(sender)
    }
}

impl Write for SenderWriter {
    fn write(&mut self, buf: &[u8]) -> Result<usize, std::io::Error> {
        futures::executor::block_on(async move {
            self.0.send(Ok(buf.to_vec())).await
                .map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err.to_string()))?;
            Ok(buf.len())
        })
    }

    fn flush(&mut self) -> Result<(), std::io::Error> {
        Ok(())
    }
}

pub struct CatalogReader<R> {
    reader: R,
}

impl <R: Read + Seek> CatalogReader<R> {

    pub fn new(reader: R) -> Self {
        Self { reader }
    }

    fn next_byte<C: Read>(mut reader: C) ->  Result<u8, std::io::Error> {
        let mut buf = [0u8; 1];
        reader.read_exact(&mut buf)?;
        Ok(buf[0])
    }

    pub fn dump(&mut self) -> Result<(), Error> {

        self.reader.seek(SeekFrom::End(-8))?;

        let start = unsafe { self.reader.read_le_value::<u64>()? };

        self.dump_dir(std::path::Path::new("./"), start)
    }

    pub fn dump_dir(&mut self, prefix: &std::path::Path, start: u64) -> Result<(), Error> {

        self.reader.seek(SeekFrom::Start(start))?;

        let size = unsafe { self.reader.read_le_value::<u32>()? } as usize;

        if size < 8 { bail!("got small directory size {}", size) };

        let data = self.reader.read_exact_allocated(size - 4)?;

        let mut cursor = &data[..];

        let entries = unsafe { cursor.read_le_value::<u32>()? };

        //println!("TEST {} {} size {}", start, entries, size);

        for _ in 0..entries {
            let etype = CatalogEntryType::try_from(Self::next_byte(&mut cursor)?)?;
            let name_len = unsafe { cursor.read_le_value::<u32>()? };
            let name = cursor.read_exact_allocated(name_len as usize)?;

            let mut path = std::path::PathBuf::from(prefix);
            path.push(std::ffi::OsString::from_vec(name));

            match etype {
                CatalogEntryType::Directory => {
                    println!("{} {:?}", char::from(etype as u8), path);
                    let offset = unsafe { cursor.read_le_value::<u64>()? };
                    if offset > start {
                        bail!("got wrong directory offset ({} > {})", offset, start);
                    }
                    let pos = start - offset;
                    self.dump_dir(&path, pos)?;
                }
                CatalogEntryType::File => {
                    let size = unsafe { cursor.read_le_value::<u64>()? };
                    let mtime = unsafe { cursor.read_le_value::<u64>()? };

                    let dt = Local.timestamp(mtime as i64, 0);

                    println!("{} {:?} {} {}",
                             char::from(etype as u8),
                             path,
                             size,
                             dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, false)
                    );
                }
                _ => {
                    println!("{} {:?}", char::from(etype as u8), path);
                }
            }
        }

        Ok(())
    }

}
