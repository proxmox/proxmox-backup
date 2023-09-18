use std::ffi::{CStr, CString, OsStr};
use std::fmt;
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::ffi::OsStrExt;

use anyhow::{bail, format_err, Error};
use serde::{Deserialize, Serialize};

use pathpatterns::{MatchList, MatchType};

use proxmox_io::ReadExt;
use proxmox_schema::api;

use crate::file_formats::PROXMOX_CATALOG_FILE_MAGIC_1_0;

/// Trait for writing file list catalogs.
///
/// A file list catalog simply stores a directory tree. Such catalogs may be used as index to do a
/// fast search for files.
pub trait BackupCatalogWriter {
    fn start_directory(&mut self, name: &CStr) -> Result<(), Error>;
    fn end_directory(&mut self) -> Result<(), Error>;
    fn add_file(&mut self, name: &CStr, size: u64, mtime: i64) -> Result<(), Error>;
    fn add_symlink(&mut self, name: &CStr) -> Result<(), Error>;
    fn add_hardlink(&mut self, name: &CStr) -> Result<(), Error>;
    fn add_block_device(&mut self, name: &CStr) -> Result<(), Error>;
    fn add_char_device(&mut self, name: &CStr) -> Result<(), Error>;
    fn add_fifo(&mut self, name: &CStr) -> Result<(), Error>;
    fn add_socket(&mut self, name: &CStr) -> Result<(), Error>;
}

#[repr(u8)]
#[derive(Copy, Clone, PartialEq, Eq)]
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
    type Error = Error;

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

impl From<&DirEntryAttribute> for CatalogEntryType {
    fn from(value: &DirEntryAttribute) -> Self {
        match value {
            DirEntryAttribute::Directory { .. } => CatalogEntryType::Directory,
            DirEntryAttribute::File { .. } => CatalogEntryType::File,
            DirEntryAttribute::Symlink => CatalogEntryType::Symlink,
            DirEntryAttribute::Hardlink => CatalogEntryType::Hardlink,
            DirEntryAttribute::BlockDevice => CatalogEntryType::BlockDevice,
            DirEntryAttribute::CharDevice => CatalogEntryType::CharDevice,
            DirEntryAttribute::Fifo => CatalogEntryType::Fifo,
            DirEntryAttribute::Socket => CatalogEntryType::Socket,
        }
    }
}

impl fmt::Display for CatalogEntryType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", char::from(*self as u8))
    }
}

/// Represents a named directory entry
///
/// The ``attr`` property contain the exact type with type specific
/// attributes.
#[derive(Clone, PartialEq, Eq)]
pub struct DirEntry {
    pub name: Vec<u8>,
    pub attr: DirEntryAttribute,
}

/// Used to specific additional attributes inside DirEntry
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DirEntryAttribute {
    Directory { start: u64 },
    File { size: u64, mtime: i64 },
    Symlink,
    Hardlink,
    BlockDevice,
    CharDevice,
    Fifo,
    Socket,
}

impl DirEntry {
    fn new(etype: CatalogEntryType, name: Vec<u8>, start: u64, size: u64, mtime: i64) -> Self {
        match etype {
            CatalogEntryType::Directory => DirEntry {
                name,
                attr: DirEntryAttribute::Directory { start },
            },
            CatalogEntryType::File => DirEntry {
                name,
                attr: DirEntryAttribute::File { size, mtime },
            },
            CatalogEntryType::Symlink => DirEntry {
                name,
                attr: DirEntryAttribute::Symlink,
            },
            CatalogEntryType::Hardlink => DirEntry {
                name,
                attr: DirEntryAttribute::Hardlink,
            },
            CatalogEntryType::BlockDevice => DirEntry {
                name,
                attr: DirEntryAttribute::BlockDevice,
            },
            CatalogEntryType::CharDevice => DirEntry {
                name,
                attr: DirEntryAttribute::CharDevice,
            },
            CatalogEntryType::Fifo => DirEntry {
                name,
                attr: DirEntryAttribute::Fifo,
            },
            CatalogEntryType::Socket => DirEntry {
                name,
                attr: DirEntryAttribute::Socket,
            },
        }
    }

    /// Get file mode bits for this entry to be used with the `MatchList` api.
    pub fn get_file_mode(&self) -> Option<u32> {
        Some(match self.attr {
            DirEntryAttribute::Directory { .. } => pxar::mode::IFDIR,
            DirEntryAttribute::File { .. } => pxar::mode::IFREG,
            DirEntryAttribute::Symlink => pxar::mode::IFLNK,
            DirEntryAttribute::Hardlink => return None,
            DirEntryAttribute::BlockDevice => pxar::mode::IFBLK,
            DirEntryAttribute::CharDevice => pxar::mode::IFCHR,
            DirEntryAttribute::Fifo => pxar::mode::IFIFO,
            DirEntryAttribute::Socket => pxar::mode::IFSOCK,
        } as u32)
    }

    /// Check if DirEntry is a directory
    pub fn is_directory(&self) -> bool {
        matches!(self.attr, DirEntryAttribute::Directory { .. })
    }

    /// Check if DirEntry is a symlink
    pub fn is_symlink(&self) -> bool {
        matches!(self.attr, DirEntryAttribute::Symlink { .. })
    }
}

struct DirInfo {
    name: CString,
    entries: Vec<DirEntry>,
}

impl DirInfo {
    fn new(name: CString) -> Self {
        DirInfo {
            name,
            entries: Vec::new(),
        }
    }

    fn new_rootdir() -> Self {
        DirInfo::new(CString::new(b"/".to_vec()).unwrap())
    }

    fn encode_entry<W: Write>(writer: &mut W, entry: &DirEntry, pos: u64) -> Result<(), Error> {
        match entry {
            DirEntry {
                name,
                attr: DirEntryAttribute::Directory { start },
            } => {
                writer.write_all(&[CatalogEntryType::Directory as u8])?;
                catalog_encode_u64(writer, name.len() as u64)?;
                writer.write_all(name)?;
                catalog_encode_u64(writer, pos - start)?;
            }
            DirEntry {
                name,
                attr: DirEntryAttribute::File { size, mtime },
            } => {
                writer.write_all(&[CatalogEntryType::File as u8])?;
                catalog_encode_u64(writer, name.len() as u64)?;
                writer.write_all(name)?;
                catalog_encode_u64(writer, *size)?;
                catalog_encode_i64(writer, *mtime)?;
            }
            DirEntry {
                name,
                attr: DirEntryAttribute::Symlink,
            } => {
                writer.write_all(&[CatalogEntryType::Symlink as u8])?;
                catalog_encode_u64(writer, name.len() as u64)?;
                writer.write_all(name)?;
            }
            DirEntry {
                name,
                attr: DirEntryAttribute::Hardlink,
            } => {
                writer.write_all(&[CatalogEntryType::Hardlink as u8])?;
                catalog_encode_u64(writer, name.len() as u64)?;
                writer.write_all(name)?;
            }
            DirEntry {
                name,
                attr: DirEntryAttribute::BlockDevice,
            } => {
                writer.write_all(&[CatalogEntryType::BlockDevice as u8])?;
                catalog_encode_u64(writer, name.len() as u64)?;
                writer.write_all(name)?;
            }
            DirEntry {
                name,
                attr: DirEntryAttribute::CharDevice,
            } => {
                writer.write_all(&[CatalogEntryType::CharDevice as u8])?;
                catalog_encode_u64(writer, name.len() as u64)?;
                writer.write_all(name)?;
            }
            DirEntry {
                name,
                attr: DirEntryAttribute::Fifo,
            } => {
                writer.write_all(&[CatalogEntryType::Fifo as u8])?;
                catalog_encode_u64(writer, name.len() as u64)?;
                writer.write_all(name)?;
            }
            DirEntry {
                name,
                attr: DirEntryAttribute::Socket,
            } => {
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

    fn parse<C: FnMut(CatalogEntryType, &[u8], u64, u64, i64) -> Result<bool, Error>>(
        data: &[u8],
        mut callback: C,
    ) -> Result<(), Error> {
        let mut cursor = data;

        let entries = catalog_decode_u64(&mut cursor)?;

        let mut name_buf = vec![0u8; 4096];

        for _ in 0..entries {
            let mut buf = [0u8];
            cursor.read_exact(&mut buf)?;
            let etype = CatalogEntryType::try_from(buf[0])?;

            let name_len = catalog_decode_u64(&mut cursor)? as usize;
            if name_len >= name_buf.len() {
                bail!(
                    "directory entry name too long ({} >= {})",
                    name_len,
                    name_buf.len()
                );
            }
            let name = &mut name_buf[0..name_len];
            cursor.read_exact(name)?;

            let cont = match etype {
                CatalogEntryType::Directory => {
                    let offset = catalog_decode_u64(&mut cursor)?;
                    callback(etype, name, offset, 0, 0)?
                }
                CatalogEntryType::File => {
                    let size = catalog_decode_u64(&mut cursor)?;
                    let mtime = catalog_decode_i64(&mut cursor)?;
                    callback(etype, name, 0, size, mtime)?
                }
                _ => callback(etype, name, 0, 0, 0)?,
            };
            if !cont {
                return Ok(());
            }
        }

        if !cursor.is_empty() {
            bail!("unable to parse whole catalog data block");
        }

        Ok(())
    }
}

/// Write small catalog files
///
/// A Catalogs simply contains list of files and directories
/// (directory tree). They are use to find content without having to
/// search the real archive (which may be large). For files, they
/// include the last modification time and file size.
pub struct CatalogWriter<W> {
    writer: W,
    dirstack: Vec<DirInfo>,
    pos: u64,
}

impl<W: Write> CatalogWriter<W> {
    /// Create a new  CatalogWriter instance
    pub fn new(writer: W) -> Result<Self, Error> {
        let mut me = Self {
            writer,
            dirstack: vec![DirInfo::new_rootdir()],
            pos: 0,
        };
        me.write_all(&PROXMOX_CATALOG_FILE_MAGIC_1_0)?;
        Ok(me)
    }

    fn write_all(&mut self, data: &[u8]) -> Result<(), Error> {
        self.writer.write_all(data)?;
        self.pos += u64::try_from(data.len())?;
        Ok(())
    }

    /// Finish writing, flush all data
    ///
    /// This need to be called before drop.
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

impl<W: Write> BackupCatalogWriter for CatalogWriter<W> {
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

        let current = self
            .dirstack
            .last_mut()
            .ok_or_else(|| format_err!("outside root"))?;
        let name = name.to_bytes().to_vec();
        current.entries.push(DirEntry {
            name,
            attr: DirEntryAttribute::Directory { start },
        });

        Ok(())
    }

    fn add_file(&mut self, name: &CStr, size: u64, mtime: i64) -> Result<(), Error> {
        let dir = self
            .dirstack
            .last_mut()
            .ok_or_else(|| format_err!("outside root"))?;
        let name = name.to_bytes().to_vec();
        dir.entries.push(DirEntry {
            name,
            attr: DirEntryAttribute::File { size, mtime },
        });
        Ok(())
    }

    fn add_symlink(&mut self, name: &CStr) -> Result<(), Error> {
        let dir = self
            .dirstack
            .last_mut()
            .ok_or_else(|| format_err!("outside root"))?;
        let name = name.to_bytes().to_vec();
        dir.entries.push(DirEntry {
            name,
            attr: DirEntryAttribute::Symlink,
        });
        Ok(())
    }

    fn add_hardlink(&mut self, name: &CStr) -> Result<(), Error> {
        let dir = self
            .dirstack
            .last_mut()
            .ok_or_else(|| format_err!("outside root"))?;
        let name = name.to_bytes().to_vec();
        dir.entries.push(DirEntry {
            name,
            attr: DirEntryAttribute::Hardlink,
        });
        Ok(())
    }

    fn add_block_device(&mut self, name: &CStr) -> Result<(), Error> {
        let dir = self
            .dirstack
            .last_mut()
            .ok_or_else(|| format_err!("outside root"))?;
        let name = name.to_bytes().to_vec();
        dir.entries.push(DirEntry {
            name,
            attr: DirEntryAttribute::BlockDevice,
        });
        Ok(())
    }

    fn add_char_device(&mut self, name: &CStr) -> Result<(), Error> {
        let dir = self
            .dirstack
            .last_mut()
            .ok_or_else(|| format_err!("outside root"))?;
        let name = name.to_bytes().to_vec();
        dir.entries.push(DirEntry {
            name,
            attr: DirEntryAttribute::CharDevice,
        });
        Ok(())
    }

    fn add_fifo(&mut self, name: &CStr) -> Result<(), Error> {
        let dir = self
            .dirstack
            .last_mut()
            .ok_or_else(|| format_err!("outside root"))?;
        let name = name.to_bytes().to_vec();
        dir.entries.push(DirEntry {
            name,
            attr: DirEntryAttribute::Fifo,
        });
        Ok(())
    }

    fn add_socket(&mut self, name: &CStr) -> Result<(), Error> {
        let dir = self
            .dirstack
            .last_mut()
            .ok_or_else(|| format_err!("outside root"))?;
        let name = name.to_bytes().to_vec();
        dir.entries.push(DirEntry {
            name,
            attr: DirEntryAttribute::Socket,
        });
        Ok(())
    }
}

/// Read Catalog files
pub struct CatalogReader<R> {
    reader: R,
}

impl<R: Read + Seek> CatalogReader<R> {
    /// Create a new CatalogReader instance
    pub fn new(reader: R) -> Self {
        Self { reader }
    }

    /// Print whole catalog to stdout
    pub fn dump(&mut self) -> Result<(), Error> {
        let root = self.root()?;
        match root {
            DirEntry {
                attr: DirEntryAttribute::Directory { start },
                ..
            } => self.dump_dir(std::path::Path::new("./"), start),
            _ => bail!("unexpected root entry type, not a directory!"),
        }
    }

    /// Get the root DirEntry
    pub fn root(&mut self) -> Result<DirEntry, Error> {
        // Root dir is special
        self.reader.seek(SeekFrom::Start(0))?;
        let mut magic = [0u8; 8];
        self.reader.read_exact(&mut magic)?;
        if magic != PROXMOX_CATALOG_FILE_MAGIC_1_0 {
            bail!("got unexpected magic number for catalog");
        }
        self.reader.seek(SeekFrom::End(-8))?;
        let start = unsafe { self.reader.read_le_value::<u64>()? };
        Ok(DirEntry {
            name: b"".to_vec(),
            attr: DirEntryAttribute::Directory { start },
        })
    }

    /// Read all directory entries
    pub fn read_dir(&mut self, parent: &DirEntry) -> Result<Vec<DirEntry>, Error> {
        let start = match parent.attr {
            DirEntryAttribute::Directory { start } => start,
            _ => bail!("parent is not a directory - internal error"),
        };

        let data = self.read_raw_dirinfo_block(start)?;

        let mut entry_list = Vec::new();

        DirInfo::parse(&data, |etype, name, offset, size, mtime| {
            let entry = DirEntry::new(etype, name.to_vec(), start - offset, size, mtime);
            entry_list.push(entry);
            Ok(true)
        })?;

        Ok(entry_list)
    }

    /// Lookup a DirEntry from an absolute path
    pub fn lookup_recursive(&mut self, path: &[u8]) -> Result<DirEntry, Error> {
        let mut current = self.root()?;
        if path == b"/" {
            return Ok(current);
        }

        let components = if !path.is_empty() && path[0] == b'/' {
            &path[1..]
        } else {
            path
        }
        .split(|c| *c == b'/');

        for comp in components {
            if let Some(entry) = self.lookup(&current, comp)? {
                current = entry;
            } else {
                bail!(
                    "path {:?} not found in catalog",
                    String::from_utf8_lossy(path)
                );
            }
        }
        Ok(current)
    }

    /// Lockup a DirEntry inside a parent directory
    pub fn lookup(
        &mut self,
        parent: &DirEntry,
        filename: &[u8],
    ) -> Result<Option<DirEntry>, Error> {
        let start = match parent.attr {
            DirEntryAttribute::Directory { start } => start,
            _ => bail!("parent is not a directory - internal error"),
        };

        let data = self.read_raw_dirinfo_block(start)?;

        let mut item = None;
        DirInfo::parse(&data, |etype, name, offset, size, mtime| {
            if name != filename {
                return Ok(true);
            }

            let entry = DirEntry::new(etype, name.to_vec(), start - offset, size, mtime);
            item = Some(entry);
            Ok(false) // stop parsing
        })?;

        Ok(item)
    }

    /// Read the raw directory info block from current reader position.
    fn read_raw_dirinfo_block(&mut self, start: u64) -> Result<Vec<u8>, Error> {
        self.reader.seek(SeekFrom::Start(start))?;
        let size = catalog_decode_u64(&mut self.reader)?;
        if size < 1 {
            bail!("got small directory size {}", size)
        };
        let data = self.reader.read_exact_allocated(size as usize)?;
        Ok(data)
    }

    /// Print the content of a directory to stdout
    pub fn dump_dir(&mut self, prefix: &std::path::Path, start: u64) -> Result<(), Error> {
        let data = self.read_raw_dirinfo_block(start)?;

        DirInfo::parse(&data, |etype, name, offset, size, mtime| {
            let mut path = std::path::PathBuf::from(prefix);
            let name: &OsStr = OsStrExt::from_bytes(name);
            path.push(name);

            match etype {
                CatalogEntryType::Directory => {
                    log::info!("{} {:?}", etype, path);
                    if offset > start {
                        bail!("got wrong directory offset ({} > {})", offset, start);
                    }
                    let pos = start - offset;
                    self.dump_dir(&path, pos)?;
                }
                CatalogEntryType::File => {
                    let mut mtime_string = mtime.to_string();
                    if let Ok(s) = proxmox_time::strftime_local("%FT%TZ", mtime) {
                        mtime_string = s;
                    }

                    log::info!("{} {:?} {} {}", etype, path, size, mtime_string,);
                }
                _ => {
                    log::info!("{} {:?}", etype, path);
                }
            }

            Ok(true)
        })
    }

    /// Finds all entries matching the given match patterns and calls the
    /// provided callback on them.
    pub fn find<'a>(
        &mut self,
        parent: &DirEntry,
        file_path: &mut Vec<u8>,
        match_list: &'a impl MatchList<'a>, //&[MatchEntry],
        callback: &mut dyn FnMut(&[u8]) -> Result<(), Error>,
    ) -> Result<(), Error> {
        let file_len = file_path.len();
        for e in self.read_dir(parent)? {
            let is_dir = e.is_directory();
            file_path.truncate(file_len);
            if !e.name.starts_with(b"/") {
                file_path.reserve(e.name.len() + 1);
                file_path.push(b'/');
            }
            file_path.extend(&e.name);
            match match_list.matches(&file_path, e.get_file_mode()) {
                Ok(Some(MatchType::Exclude)) => continue,
                Ok(Some(MatchType::Include)) => callback(file_path)?,
                _ => (),
            }
            if is_dir {
                self.find(&e, file_path, match_list, callback)?;
            }
        }
        file_path.truncate(file_len);

        Ok(())
    }

    /// Returns the list of content of the given path
    pub fn list_dir_contents(&mut self, path: &[u8]) -> Result<Vec<ArchiveEntry>, Error> {
        let dir = self.lookup_recursive(path)?;
        let mut res = vec![];
        let mut path = path.to_vec();
        if !path.is_empty() && path[0] == b'/' {
            path.remove(0);
        }

        for direntry in self.read_dir(&dir)? {
            let mut components = path.clone();
            components.push(b'/');
            components.extend(&direntry.name);
            let mut entry = ArchiveEntry::new(&components, Some(&direntry.attr));
            if let DirEntryAttribute::File { size, mtime } = direntry.attr {
                entry.size = size.into();
                entry.mtime = mtime.into();
            }
            res.push(entry);
        }

        Ok(res)
    }
}

/// Serialize i64 as short, variable length byte sequence
///
/// Stores 7 bits per byte, Bit 8 indicates the end of the sequence (when not set).
/// If the value is negative, we end with a zero byte (0x00).
#[allow(clippy::neg_multiply)]
pub fn catalog_encode_i64<W: Write>(writer: &mut W, v: i64) -> Result<(), Error> {
    let mut enc = Vec::new();

    let mut d = if v < 0 {
        (-1 * (v + 1)) as u64 + 1 // also handles i64::MIN
    } else {
        v as u64
    };

    loop {
        if d < 128 {
            if v < 0 {
                enc.push(128 | d as u8);
                enc.push(0u8);
            } else {
                enc.push(d as u8);
            }
            break;
        }
        enc.push((128 | (d & 127)) as u8);
        d >>= 7;
    }
    writer.write_all(&enc)?;

    Ok(())
}

/// Deserialize i64 from variable length byte sequence
///
/// We currently read maximal 11 bytes, which give a maximum of 70 bits + sign.
/// this method is compatible with catalog_encode_u64 iff the
/// value encoded is <= 2^63 (values > 2^63 cannot be represented in an i64)
#[allow(clippy::neg_multiply)]
pub fn catalog_decode_i64<R: Read>(reader: &mut R) -> Result<i64, Error> {
    let mut v: u64 = 0;
    let mut buf = [0u8];

    for i in 0..11 {
        // only allow 11 bytes (70 bits + sign marker)
        if buf.is_empty() {
            bail!("decode_i64 failed - unexpected EOB");
        }
        reader.read_exact(&mut buf)?;

        let t = buf[0];

        if t == 0 {
            if v == 0 {
                return Ok(0);
            }
            return Ok(((v - 1) as i64 * -1) - 1); // also handles i64::MIN
        } else if t < 128 {
            v |= (t as u64) << (i * 7);
            return Ok(v as i64);
        } else {
            v |= ((t & 127) as u64) << (i * 7);
        }
    }

    bail!("decode_i64 failed - missing end marker");
}

/// Serialize u64 as short, variable length byte sequence
///
/// Stores 7 bits per byte, Bit 8 indicates the end of the sequence (when not set).
pub fn catalog_encode_u64<W: Write>(writer: &mut W, v: u64) -> Result<(), Error> {
    let mut enc = Vec::new();

    let mut d = v;
    loop {
        if d < 128 {
            enc.push(d as u8);
            break;
        }
        enc.push((128 | (d & 127)) as u8);
        d >>= 7;
    }
    writer.write_all(&enc)?;

    Ok(())
}

/// Deserialize u64 from variable length byte sequence
///
/// We currently read maximal 10 bytes, which give a maximum of 70 bits,
/// but we currently only encode up to 64 bits
pub fn catalog_decode_u64<R: Read>(reader: &mut R) -> Result<u64, Error> {
    let mut v: u64 = 0;
    let mut buf = [0u8];

    for i in 0..10 {
        // only allow 10 bytes (70 bits)
        if buf.is_empty() {
            bail!("decode_u64 failed - unexpected EOB");
        }
        reader.read_exact(&mut buf)?;
        let t = buf[0];
        if t < 128 {
            v |= (t as u64) << (i * 7);
            return Ok(v);
        } else {
            v |= ((t & 127) as u64) << (i * 7);
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

    test_encode_decode(u64::MIN);
    test_encode_decode(126);
    test_encode_decode((1 << 12) - 1);
    test_encode_decode((1 << 20) - 1);
    test_encode_decode((1 << 50) - 1);
    test_encode_decode(u64::MAX);
}

#[test]
fn test_catalog_i64_encoder() {
    fn test_encode_decode(value: i64) {
        let mut data = Vec::new();
        catalog_encode_i64(&mut data, value).unwrap();

        let slice = &mut &data[..];
        let decoded = catalog_decode_i64(slice).unwrap();

        assert!(decoded == value);
    }

    test_encode_decode(0);
    test_encode_decode(-0);
    test_encode_decode(126);
    test_encode_decode(-126);
    test_encode_decode((1 << 12) - 1);
    test_encode_decode(-(1 << 12) - 1);
    test_encode_decode((1 << 20) - 1);
    test_encode_decode(-(1 << 20) - 1);
    test_encode_decode(i64::MIN);
    test_encode_decode(i64::MAX);
}

#[test]
fn test_catalog_i64_compatibility() {
    fn test_encode_decode(value: u64) {
        let mut data = Vec::new();
        catalog_encode_u64(&mut data, value).unwrap();

        let slice = &mut &data[..];
        let decoded = catalog_decode_i64(slice).unwrap() as u64;

        assert!(decoded == value);
    }

    test_encode_decode(u64::MIN);
    test_encode_decode(126);
    test_encode_decode((1 << 12) - 1);
    test_encode_decode((1 << 20) - 1);
    test_encode_decode((1 << 50) - 1);
    test_encode_decode(u64::MAX);
}

/// An entry in a hierarchy of files for restore and listing.
#[api]
#[derive(Serialize, Deserialize)]
pub struct ArchiveEntry {
    /// Base64-encoded full path to the file, including the filename
    pub filepath: String,
    /// Displayable filename text for UIs
    pub text: String,
    /// File or directory type of this entry
    #[serde(rename = "type")]
    pub entry_type: String,
    /// Is this entry a leaf node, or does it have children (i.e. a directory)?
    pub leaf: bool,
    /// The file size, if entry_type is 'f' (file)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    /// The file "last modified" time stamp, if entry_type is 'f' (file)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mtime: Option<i64>,
}

impl ArchiveEntry {
    pub fn new(filepath: &[u8], entry_type: Option<&DirEntryAttribute>) -> Self {
        let size = match entry_type {
            Some(DirEntryAttribute::File { size, .. }) => Some(*size),
            _ => None,
        };
        Self::new_with_size(filepath, entry_type, size)
    }

    pub fn new_with_size(
        filepath: &[u8],
        entry_type: Option<&DirEntryAttribute>,
        size: Option<u64>,
    ) -> Self {
        Self {
            filepath: base64::encode(filepath),
            text: String::from_utf8_lossy(filepath.split(|x| *x == b'/').last().unwrap())
                .to_string(),
            entry_type: match entry_type {
                Some(entry_type) => CatalogEntryType::from(entry_type).to_string(),
                None => "v".to_owned(),
            },
            leaf: !matches!(entry_type, None | Some(DirEntryAttribute::Directory { .. })),
            size,
            mtime: match entry_type {
                Some(DirEntryAttribute::File { mtime, .. }) => Some(*mtime),
                _ => None,
            },
        }
    }
}
