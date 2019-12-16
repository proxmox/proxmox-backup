//! *pxar* format decoder for seekable files
//!
//! This module contain the code to decode *pxar* archive files.

use std::convert::TryFrom;
use std::ffi::{OsString, OsStr};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::os::unix::ffi::OsStrExt;

use failure::*;
use libc;

use super::binary_search_tree::search_binary_tree_by;
use super::format_definition::*;
use super::sequential_decoder::SequentialDecoder;
use super::match_pattern::MatchPattern;

use proxmox::tools::io::ReadExt;

pub struct DirectoryEntry {
    /// Points to the `PxarEntry` of the directory
    start: u64,
    /// Points past the goodbye table tail
    end: u64,
    pub filename: OsString,
    pub entry: PxarEntry,
}

/// Trait to create ReadSeek Decoder trait objects.
trait ReadSeek: Read + Seek {}
impl <R: Read + Seek> ReadSeek for R {}

// This one needs Read+Seek
pub struct Decoder {
    inner: SequentialDecoder<Box<dyn ReadSeek + Send>>,
    root_start: u64,
    root_end: u64,
}

const HEADER_SIZE: u64 = std::mem::size_of::<PxarHeader>() as u64;
const GOODBYE_ITEM_SIZE: u64 = std::mem::size_of::<PxarGoodbyeItem>() as u64;

impl Decoder {
    pub fn new<R: Read + Seek + Send + 'static>(mut reader: R) -> Result<Self, Error> {
        let root_end = reader.seek(SeekFrom::End(0))?;
        let boxed_reader: Box<dyn ReadSeek + 'static + Send> = Box::new(reader);
        let inner = SequentialDecoder::new(boxed_reader, super::flags::DEFAULT);
  
        Ok(Self { inner, root_start: 0, root_end })
    }

    pub fn set_callback<F: Fn(&Path) -> Result<(), Error> + Send + 'static>(&mut self, callback: F ) {
        self.inner.set_callback(callback);
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

    /// Restore the subarchive starting at `dir` to the provided target `path`.
    ///
    /// Only restore the content matched by the MatchPattern `pattern`.
    /// An empty Vec `pattern` means restore all.
    pub fn restore(&mut self, dir: &DirectoryEntry, path: &Path, pattern: &Vec<MatchPattern>) -> Result<(), Error> {
        let start = dir.start;
        self.seek(SeekFrom::Start(start))?;
        self.inner.restore(path, pattern)?;

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

    /// Lookup the item identified by `filename` in the provided `DirectoryEntry`.
    ///
    /// Calculates the hash of the filename and searches for matching entries in
    /// the goodbye table of the provided `DirectoryEntry`.
    /// If found, also the filename is compared to avoid hash collision.
    /// If the filename does not match, the search resumes with the next entry in
    /// the goodbye table.
    /// If there is no entry with matching `filename`, `Ok(None)` is returned.
    pub fn lookup(
        &mut self,
        dir: &DirectoryEntry,
        filename: &OsStr,
    ) -> Result<Option<(DirectoryEntry, PxarAttributes, u64)>, Error> {
        let gbt = self.goodbye_table(Some(dir.start), dir.end)?;
        let hash = compute_goodbye_hash(filename.as_bytes());

        let mut start_idx = 0;
        let mut skip_multiple = 0;
        loop {
            // Search for the next goodbye entry with matching hash.
            let idx = search_binary_tree_by(
                start_idx,
                gbt.len(),
                skip_multiple,
                |idx| hash.cmp(&gbt[idx].0.hash),
            );
            let (_item, start, end) = match idx {
                Some(idx) => &gbt[idx],
                None => return Ok(None),
            };

            // At this point it is not clear if the item is a directory or not,
            // this has to be decided based on the entry mode.
            // `Decoder`s attributes function accepts both, offsets pointing to
            // the start of an item (PXAR_FILENAME) or the GOODBYE_TAIL_MARKER in
            // case of directories, so the use of start offset is fine for both
            // cases.
            let (entry_name, entry, attr, payload_size) = self.attributes(*start)?;

            // Possible hash collision, need to check if the found entry is indeed
            // the filename to lookup.
            if entry_name == filename {
                let dir_entry = DirectoryEntry {
                    start: *start + HEADER_SIZE + entry_name.len() as u64 + 1,
                    end: *end,
                    filename: entry_name,
                    entry,
                };
                return Ok(Some((dir_entry, attr, payload_size)));
            }
            // Hash collision, check the next entry in the goodbye table by starting
            // from given index but skipping one more match (so hash at index itself).
            start_idx = idx.unwrap();
            skip_multiple = 1;
        }
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
            // Make sure to return the original filename,
            // not the one read from the hardlink.
            let (_, entry, xattr, file_size) = self.attributes(offset - diff)?;
            return Ok((filename, entry, xattr, file_size));
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
        if head.htype == PXAR_FORMAT_HARDLINK {
            let (_, diff) = self.inner.read_hardlink(head.size)?;
            return self.read(offset - diff, size, data_offset);
        }
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
