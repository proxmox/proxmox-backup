//! *pxar* format decoder for seekable files
//!
//! This module contain the code to decode *pxar* archive files.

use std::convert::TryFrom;
use std::ffi::OsString;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use failure::*;
use libc;

use super::format_definition::*;
use super::sequential_decoder::*;

use proxmox::tools::io::ReadExt;

pub struct DirectoryEntry {
    start: u64,
    end: u64,
    pub filename: OsString,
    pub entry: PxarEntry,
}

// This one needs Read+Seek
pub struct Decoder<R: Read + Seek, F: Fn(&Path) -> Result<(), Error>> {
    inner: SequentialDecoder<R, F>,
    root_start: u64,
    root_end: u64,
}

const HEADER_SIZE: u64 = std::mem::size_of::<PxarHeader>() as u64;
const GOODBYE_ITEM_SIZE: u64 = std::mem::size_of::<PxarGoodbyeItem>() as u64;

impl<R: Read + Seek, F: Fn(&Path) -> Result<(), Error>> Decoder<R, F> {
    pub fn new(mut reader: R, callback: F) -> Result<Self, Error> {
        let root_end = reader.seek(SeekFrom::End(0))?;

        Ok(Self {
            inner: SequentialDecoder::new(reader, super::flags::DEFAULT, callback),
            root_start: 0,
            root_end,
        })
    }

    pub fn root(&mut self) -> Result<DirectoryEntry, Error> {
        self.seek(SeekFrom::Start(0))?;
        let header: PxarHeader = self.inner.read_item()?;
        check_ca_header::<PxarEntry>(&header, PXAR_ENTRY)?;
        let entry: PxarEntry = self.inner.read_item()?;
        Ok(DirectoryEntry {
            start: self.root_start,
            end: self.root_end,
            filename: OsString::new(), // Empty
            entry,
        })
    }

    fn seek(&mut self, pos: SeekFrom) -> Result<u64, Error> {
        let pos = self.inner.get_reader_mut().seek(pos)?;
        Ok(pos)
    }

    pub(crate) fn root_end_offset(&self) -> u64 {
        self.root_end
    }

    pub fn restore(&mut self, dir: &DirectoryEntry, path: &Path) -> Result<(), Error> {
        let start = dir.start;

        self.seek(SeekFrom::Start(start))?;

        self.inner.restore(path, &Vec::new())?;

        Ok(())
    }

    pub(crate) fn read_directory_entry(
        &mut self,
        start: u64,
        end: u64,
    ) -> Result<DirectoryEntry, Error> {
        self.seek(SeekFrom::Start(start))?;

        let head: PxarHeader = self.inner.read_item()?;

        if head.htype != PXAR_FILENAME {
            bail!("wrong filename header type for object [{}..{}]", start, end);
        }

        let entry_start = start + head.size;

        let filename = self.inner.read_filename(head.size)?;

        let head: PxarHeader = self.inner.read_item()?;
        if head.htype == PXAR_FORMAT_HARDLINK {
            let (_, offset) = self.inner.read_hardlink(head.size)?;
            // TODO: Howto find correct end offset for hardlink target?
            // This is a bit tricky since we cannot find correct end in an efficient
            // way, on the other hand it doesn't really matter (for now) since target
            // is never a directory and end is not used in such cases.
            return self.read_directory_entry(start - offset, end);
        }
        check_ca_header::<PxarEntry>(&head, PXAR_ENTRY)?;
        let entry: PxarEntry = self.inner.read_item()?;

        Ok(DirectoryEntry {
            start: entry_start,
            end,
            filename,
            entry,
        })
    }

    /// Return the goodbye table based on the provided end offset.
    ///
    /// Get the goodbye table entries and the start and end offsets of the
    /// items they reference.
    /// If the start offset is provided, we use that to check the consistency of
    /// the data, else the start offset calculated based on the goodbye tail is
    /// used.
    pub(crate) fn goodbye_table(
        &mut self,
        start: Option<u64>,
        end: u64,
    ) -> Result<Vec<(PxarGoodbyeItem, u64, u64)>, Error> {
        self.seek(SeekFrom::Start(end - GOODBYE_ITEM_SIZE))?;

        let tail: PxarGoodbyeItem = self.inner.read_item()?;
        if tail.hash != PXAR_GOODBYE_TAIL_MARKER {
            bail!("missing goodbye tail marker for object at offset {}", end);
        }

        // If the start offset was provided, we use and check based on that.
        // If not, we rely on the offset calculated from the goodbye table entry.
        let start = start.unwrap_or(end - tail.offset - tail.size);
        let goodbye_table_size = tail.size;
        if goodbye_table_size < (HEADER_SIZE + GOODBYE_ITEM_SIZE) {
            bail!("short goodbye table size for object [{}..{}]", start, end);
        }

        let goodbye_inner_size = goodbye_table_size - HEADER_SIZE - GOODBYE_ITEM_SIZE;
        if (goodbye_inner_size % GOODBYE_ITEM_SIZE) != 0 {
            bail!(
                "wrong goodbye inner table size for entry [{}..{}]",
                start,
                end
            );
        }

        let goodbye_start = end - goodbye_table_size;
        if tail.offset != (goodbye_start - start) {
            bail!(
                "wrong offset in goodbye tail marker for entry [{}..{}]",
                start,
                end
            );
        }

        self.seek(SeekFrom::Start(goodbye_start))?;
        let head: PxarHeader = self.inner.read_item()?;
        if head.htype != PXAR_GOODBYE {
            bail!(
                "wrong goodbye table header type for entry [{}..{}]",
                start,
                end
            );
        }

        if head.size != goodbye_table_size {
            bail!("wrong goodbye table size for entry [{}..{}]", start, end);
        }

        let mut gb_entries = Vec::new();
        for i in 0..goodbye_inner_size / GOODBYE_ITEM_SIZE {
            let item: PxarGoodbyeItem = self.inner.read_item()?;
            if item.offset > (goodbye_start - start) {
                bail!(
                    "goodbye entry {} offset out of range [{}..{}] {} {} {}",
                    i,
                    start,
                    end,
                    item.offset,
                    goodbye_start,
                    start
                );
            }
            let item_start = goodbye_start - item.offset;
            let item_end = item_start + item.size;
            if item_end > goodbye_start {
                bail!("goodbye entry {} end out of range [{}..{}]", i, start, end);
            }
            gb_entries.push((item, item_start, item_end));
        }

        Ok(gb_entries)
    }

    pub fn list_dir(&mut self, dir: &DirectoryEntry) -> Result<Vec<DirectoryEntry>, Error> {
        let start = dir.start;
        let end = dir.end;

        //println!("list_dir1: {} {}", start, end);

        if (end - start) < (HEADER_SIZE + GOODBYE_ITEM_SIZE) {
            bail!("detected short object [{}..{}]", start, end);
        }

        let mut result = vec![];
        let goodbye_table = self.goodbye_table(Some(start), end)?;
        for (_, item_start, item_end) in goodbye_table {
            let entry = self.read_directory_entry(item_start, item_end)?;
            //println!("ENTRY: {} {} {:?}", item_start, item_end, entry.filename);
            result.push(entry);
        }

        Ok(result)
    }

    pub fn print_filenames<W: std::io::Write>(
        &mut self,
        output: &mut W,
        prefix: &mut PathBuf,
        dir: &DirectoryEntry,
    ) -> Result<(), Error> {
        let mut list = self.list_dir(dir)?;

        list.sort_unstable_by(|a, b| a.filename.cmp(&b.filename));

        for item in &list {
            prefix.push(item.filename.clone());

            let mode = item.entry.mode as u32;

            let ifmt = mode & libc::S_IFMT;

            writeln!(output, "{:?}", prefix)?;

            match ifmt {
                libc::S_IFDIR => self.print_filenames(output, prefix, item)?,
                libc::S_IFREG | libc::S_IFLNK | libc::S_IFBLK | libc::S_IFCHR => {}
                _ => bail!("unknown item mode/type for {:?}", prefix),
            }

            prefix.pop();
        }

        Ok(())
    }

    /// Get the `DirectoryEntry` located at `offset`.
    ///
    /// `offset` is expected to point to the directories `PXAR_GOODBYE_TAIL_MARKER`.
    pub fn get_dir(&mut self, offset: u64) -> Result<DirectoryEntry, Error> {
        self.seek(SeekFrom::Start(offset))?;

        let gb: PxarGoodbyeItem = self.inner.read_item()?;
        if gb.hash != PXAR_GOODBYE_TAIL_MARKER {
            bail!("Expected goodbye tail marker, encountered 0x{:x?}", gb.hash);
        }

        let distance = i64::try_from(gb.offset + gb.size)?;
        let start = self.seek(SeekFrom::Current(0 - distance))?;
        let mut header: PxarHeader = self.inner.read_item()?;
        let filename = if header.htype == PXAR_FILENAME {
            let name = self.inner.read_filename(header.size)?;
            header = self.inner.read_item()?;
            name
        } else {
            OsString::new()
        };
        check_ca_header::<PxarEntry>(&header, PXAR_ENTRY)?;
        let entry: PxarEntry = self.inner.read_item()?;

        Ok(DirectoryEntry {
            start,
            end: offset + GOODBYE_ITEM_SIZE,
            filename,
            entry,
        })
    }

    /// Get attributes for the archive item located at `offset`.
    ///
    /// Returns the entry, attributes and the payload size for the item.
    /// For regular archive itmes a `PXAR_FILENAME` or a `PXAR_ENTRY` header is
    /// expected at `offset`.
    /// For directories, `offset` might also (but not necessarily) point at the
    /// directories `PXAR_GOODBYE_TAIL_MARKER`. This is not mandatory and it can
    /// also directly point to its `PXAR_FILENAME` or `PXAR_ENTRY`, thereby
    /// avoiding an additional seek.
    pub fn attributes(&mut self, offset: u64) -> Result<(OsString, PxarEntry, PxarAttributes, u64), Error> {
        self.seek(SeekFrom::Start(offset))?;

        let mut marker: u64 = self.inner.read_item()?;
        if marker == PXAR_GOODBYE_TAIL_MARKER {
            let dir_offset: u64 = self.inner.read_item()?;
            let gb_size: u64 = self.inner.read_item()?;
            let distance = i64::try_from(dir_offset + gb_size)?;
            self.seek(SeekFrom::Current(0 - distance))?;
            marker = self.inner.read_item()?;
        }

        let filename = if marker == PXAR_FILENAME {
            let size: u64 = self.inner.read_item()?;
            let filename = self.inner.read_filename(size)?;
            marker = self.inner.read_item()?;
            filename
        } else {
            OsString::new()
        };

        if marker == PXAR_FORMAT_HARDLINK {
            let size: u64 = self.inner.read_item()?;
            let (_, diff) = self.inner.read_hardlink(size)?;
            return self.attributes(offset - diff);
        }

        if marker != PXAR_ENTRY {
            bail!("Expected PXAR_ENTRY, found 0x{:x?}", marker);
        }
        let _size: u64 = self.inner.read_item()?;
        let entry: PxarEntry = self.inner.read_item()?;
        let (header, xattr) = self.inner.read_attributes()?;
        let file_size = match header.htype {
            PXAR_PAYLOAD => header.size - HEADER_SIZE,
            _ => 0,
        };

        Ok((filename, entry, xattr, file_size))
    }

    /// Opens the file by validating the given `offset` and returning its attrs,
    /// xattrs and size.
    pub fn open(&mut self, offset: u64) -> Result<(OsString, PxarEntry, PxarAttributes, u64), Error> {
        self.attributes(offset)
    }

    /// Read the payload of the file given by `offset`.
    ///
    /// This will read the file by first seeking to `offset` within the archive,
    /// check if there is indeed a valid item with payload and then read `size`
    /// bytes of content starting from `data_offset`.
    /// If EOF is reached before reading `size` bytes, the reduced buffer is
    /// returned.
    pub fn read(&mut self, offset: u64, size: usize, data_offset: u64) -> Result<Vec<u8>, Error> {
        self.seek(SeekFrom::Start(offset))?;
        let head: PxarHeader = self.inner.read_item()?;
        if head.htype != PXAR_FILENAME {
            bail!("Expected PXAR_FILENAME, encountered 0x{:x?}", head.htype);
        }
        let _filename = self.inner.read_filename(head.size)?;

        let head: PxarHeader = self.inner.read_item()?;
        check_ca_header::<PxarEntry>(&head, PXAR_ENTRY)?;
        let _: PxarEntry = self.inner.read_item()?;

        let (header, _) = self.inner.read_attributes()?;
        if header.htype != PXAR_PAYLOAD {
            bail!("Expected PXAR_PAYLOAD, found 0x{:x?}", header.htype);
        }

        let payload_size = header.size - HEADER_SIZE;
        if data_offset >= payload_size {
            return Ok(Vec::new());
        }

        let len = if data_offset + u64::try_from(size)? > payload_size {
            usize::try_from(payload_size - data_offset)?
        } else {
            size
        };
        self.inner.skip_bytes(usize::try_from(data_offset)?)?;
        let data = self.inner.get_reader_mut().read_exact_allocated(len)?;

        Ok(data)
    }

    /// Read the target of a hardlink in the archive.
    pub fn read_link(&mut self, offset: u64) -> Result<(PathBuf, PxarEntry), Error> {
        self.seek(SeekFrom::Start(offset))?;
        let mut header: PxarHeader = self.inner.read_item()?;
        if header.htype != PXAR_FILENAME {
            bail!("Expected PXAR_FILENAME, encountered 0x{:x?}", header.htype);
        }
        let _filename = self.inner.read_filename(header.size)?;

        header = self.inner.read_item()?;
        check_ca_header::<PxarEntry>(&header, PXAR_ENTRY)?;
        let entry: PxarEntry = self.inner.read_item()?;

        header = self.inner.read_item()?;
        if header.htype != PXAR_SYMLINK {
            bail!("Expected PXAR_SYMLINK, encountered 0x{:x?}", header.htype);
        }
        let target = self.inner.read_link(header.size)?;

        Ok((target, entry))
    }
}
