use failure::*;
use std::ffi::{CStr, CString};
use std::os::unix::ffi::OsStringExt;
use std::io::{Read, Write, Seek, SeekFrom};
use std::convert::TryFrom;

use chrono::offset::{TimeZone, Local};

use proxmox::tools::io::ReadExt;

use crate::pxar::catalog::{BackupCatalogWriter, CatalogEntryType};
use crate::backup::file_formats::PROXMOX_CATALOG_FILE_MAGIC_1_0;

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

    fn encode_entry<W: Write>(
        writer: &mut W,
        entry: &DirEntry,
        pos: u64,
    ) -> Result<(), Error> {
        match entry {
            DirEntry::Directory { name, start } => {
                writer.write_all(&[CatalogEntryType::Directory as u8])?;
                catalog_encode_u64(writer, name.len() as u64)?;
                writer.write_all(name)?;
                catalog_encode_u64(writer, pos - start)?;
            }
            DirEntry::File { name, size, mtime } => {
                writer.write_all(&[CatalogEntryType::File as u8])?;
                catalog_encode_u64(writer, name.len() as u64)?;
                writer.write_all(name)?;
                catalog_encode_u64(writer, *size)?;
                catalog_encode_u64(writer, *mtime)?;
            }
            DirEntry::Symlink { name } => {
                writer.write_all(&[CatalogEntryType::Symlink as u8])?;
                catalog_encode_u64(writer, name.len() as u64)?;
                writer.write_all(name)?;
            }
            DirEntry::Hardlink { name } => {
                writer.write_all(&[CatalogEntryType::Hardlink as u8])?;
                catalog_encode_u64(writer, name.len() as u64)?;
                writer.write_all(name)?;
            }
            DirEntry::BlockDevice { name } => {
                writer.write_all(&[CatalogEntryType::BlockDevice as u8])?;
                catalog_encode_u64(writer, name.len() as u64)?;
                writer.write_all(name)?;
            }
            DirEntry::CharDevice { name } => {
                writer.write_all(&[CatalogEntryType::CharDevice as u8])?;
                catalog_encode_u64(writer, name.len() as u64)?;
                writer.write_all(name)?;
            }
            DirEntry::Fifo { name } => {
                writer.write_all(&[CatalogEntryType::Fifo as u8])?;
                catalog_encode_u64(writer, name.len() as u64)?;
                writer.write_all(name)?;
            }
            DirEntry::Socket { name } => {
                writer.write_all(&[CatalogEntryType::Socket as u8])?;
                catalog_encode_u64(writer, name.len() as u64)?;
                writer.write_all(name)?;
            }
        }
        Ok(())
    }

    fn encode(self, start: u64) -> Result<(CString, Vec<u8>), Error> {
        let mut table = Vec::new();
        catalog_encode_u64(&mut table, self.entries.len() as u64)?;
        for entry in self.entries {
            Self::encode_entry(&mut table, &entry, start)?;
        }

        let mut data = Vec::new();
        catalog_encode_u64(&mut data, table.len() as u64)?;
        data.extend_from_slice(&table);

        Ok((self.name, data))
    }

    fn parse<C: FnMut(CatalogEntryType, Vec<u8>, u64, u64, u64) -> Result<(), Error>>(
        data: &[u8],
        mut callback: C,
    ) -> Result<(), Error> {

        let mut cursor = data;

        let entries = catalog_decode_u64(&mut cursor)?;

        for _ in 0..entries {

            let mut buf = [ 0u8 ];
            cursor.read_exact(&mut buf)?;
            let etype = CatalogEntryType::try_from(buf[0])?;

            let name_len = catalog_decode_u64(&mut cursor)?;
            let name = cursor.read_exact_allocated(name_len as usize)?;

            match etype {
                CatalogEntryType::Directory => {
                    let offset = catalog_decode_u64(&mut cursor)?;
                    callback(etype, name, offset, 0, 0)?;
                }
                CatalogEntryType::File => {
                    let size = catalog_decode_u64(&mut cursor)?;
                    let mtime = catalog_decode_u64(&mut cursor)?;
                    callback(etype, name, 0, size, mtime)?;
                }
                _ => {
                    callback(etype, name, 0, 0, 0)?;
                }
            }
        }

        if !cursor.is_empty() {
            bail!("unable to parse whole catalog data block");
        }

        Ok(())
    }
}

pub struct CatalogWriter<W> {
    writer: W,
    dirstack: Vec<DirInfo>,
    pos: u64,
}

impl <W: Write> CatalogWriter<W> {

    pub fn new(writer: W) -> Result<Self, Error> {
        let mut me = Self { writer, dirstack: vec![ DirInfo::new_rootdir() ], pos: 0 };
        me.write_all(&PROXMOX_CATALOG_FILE_MAGIC_1_0)?;
        Ok(me)
    }

    fn write_all(&mut self, data: &[u8]) -> Result<(), Error> {
        self.writer.write_all(data)?;
        self.pos += u64::try_from(data.len())?;
        Ok(())
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

    pub fn dump(&mut self) -> Result<(), Error> {

        self.reader.seek(SeekFrom::End(-8))?;

        let start = unsafe { self.reader.read_le_value::<u64>()? };

        self.dump_dir(std::path::Path::new("./"), start)
    }

    pub fn dump_dir(&mut self, prefix: &std::path::Path, start: u64) -> Result<(), Error> {

        self.reader.seek(SeekFrom::Start(start))?;

        let size = catalog_decode_u64(&mut self.reader)?;

        if size < 1 { bail!("got small directory size {}", size) };

        let data = self.reader.read_exact_allocated(size as usize)?;

        DirInfo::parse(&data, |etype, name, offset, size, mtime| {

            let mut path = std::path::PathBuf::from(prefix);
            path.push(std::ffi::OsString::from_vec(name));

            match etype {
                CatalogEntryType::Directory => {
                    println!("{} {:?}", etype, path);
                    if offset > start {
                        bail!("got wrong directory offset ({} > {})", offset, start);
                    }
                    let pos = start - offset;
                    self.dump_dir(&path, pos)?;
                }
                CatalogEntryType::File => {
                    let dt = Local.timestamp(mtime as i64, 0);

                    println!(
                        "{} {:?} {} {}",
                        etype,
                        path,
                        size,
                        dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, false),
                    );
                }
                _ => {
                    println!("{} {:?}", etype, path);
                }
            }

            Ok(())
        })
    }
}

/// Serialize u64 as short, variable length byte sequence
///
/// Stores 7 bits per byte, Bit 8 indicates the end of the sequence (when not set).
/// We limit values to a maximum of 2^63.
pub fn catalog_encode_u64<W: Write>(writer: &mut W, v: u64) -> Result<(), Error> {
    let mut enc = Vec::new();

    if (v & (1<<63)) != 0 { bail!("catalog_encode_u64 failed - value >= 2^63"); }
    let mut d = v;
    loop {
        if d < 128 {
            enc.push(d as u8);
            break;
        }
        enc.push((128 | (d & 127)) as u8);
        d = d >> 7;
    }
    writer.write_all(&enc)?;

    Ok(())
}

/// Deserialize u64 from variable length byte sequence
///
/// We currently read maximal 9 bytes, which give a maximum of 63 bits.
pub fn catalog_decode_u64<R: Read>(reader: &mut R) -> Result<u64, Error> {

    let mut v: u64 = 0;
    let mut buf = [0u8];

    for i in 0..9 { // only allow 9 bytes (63 bits)
        if buf.is_empty() {
            bail!("decode_u64 failed - unexpected EOB");
        }
        reader.read_exact(&mut buf)?;
        let t = buf[0];
        if t < 128 {
            v |= (t as u64) << (i*7);
            return Ok(v);
        } else {
            v |= ((t & 127) as u64) << (i*7);
        }
    }

    bail!("decode_u64 failed - missing end marker");
}

#[test]
fn test_catalog_u64_encoder() {

    fn test_encode_decode(value: u64) {

        let mut data = Vec::new();
        catalog_encode_u64(&mut data, value).unwrap();

        //println!("ENCODE {} {:?}", value, data);

        let slice = &mut &data[..];
        let decoded = catalog_decode_u64(slice).unwrap();

        //println!("DECODE {}", decoded);

        assert!(decoded == value);
    }

    test_encode_decode(126);
    test_encode_decode((1<<12)-1);
    test_encode_decode((1<<20)-1);
    test_encode_decode((1<<50)-1);
    test_encode_decode((1<<63)-1);
}
