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
    /// Filename of entry
    pub filename: OsString,
    /// Entry (mode, permissions)
    pub entry: PxarEntry,
    /// Extended attributes
    pub xattr: PxarAttributes,
    /// Payload size
    pub size: u64,
    /// Target path for symbolic links
    pub target: Option<PathBuf>,
    /// Start offset of the payload if present.
    pub payload_offset: Option<u64>,
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
        let (header, xattr) = self.inner.read_attributes()?;
        let (size, payload_offset) = match header.htype {
            PXAR_PAYLOAD => (header.size - HEADER_SIZE, Some(self.seek(SeekFrom::Current(0))?)),
            _ => (0, None),
        };

        Ok(DirectoryEntry {
            start: self.root_start,
            end: self.root_end,
            filename: OsString::new(), // Empty
            entry,
            xattr,
            size,
            target: None,
            payload_offset,
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
        let (header, xattr) = self.inner.read_attributes()?;
        let (size, payload_offset, target) = match header.htype {
            PXAR_PAYLOAD =>
                (header.size - HEADER_SIZE, Some(self.seek(SeekFrom::Current(0))?), None),
            PXAR_SYMLINK =>
                (header.size - HEADER_SIZE, None, Some(self.inner.read_link(header.size)?)),
            _ => (0, None, None),
        };

        Ok(DirectoryEntry {
            start: entry_start,
            end,
            filename,
            entry,
            xattr,
            size,
            target,
            payload_offset,
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
    ) -> Result<Option<DirectoryEntry>, Error> {
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

            let entry = self.read_directory_entry(*start, *end)?;

            // Possible hash collision, need to check if the found entry is indeed
            // the filename to lookup.
            if entry.filename == filename {
                return Ok(Some(entry));
            }
            // Hash collision, check the next entry in the goodbye table by starting
            // from given index but skipping one more match (so hash at index itself).
            start_idx = idx.unwrap();
            skip_multiple = 1;
        }
    }

    /// Read the payload of the file given by `entry`.
    ///
    /// This will read a files payload as raw bytes starting from `offset` after
    /// the payload marker, reading `size` bytes.
    /// If the payload from `offset` to EOF is smaller than `size` bytes, the
    /// buffer with reduced size is returned.
    /// If `offset` is larger than the payload size of the `DirectoryEntry`, an
    /// empty buffer is returned.
    pub fn read(&mut self, entry: &DirectoryEntry, size: usize, offset: u64) -> Result<Vec<u8>, Error> {
        let start_offset = entry.payload_offset
            .ok_or_else(|| format_err!("entry has no payload offset"))?;
        if offset >= entry.size {
            return Ok(Vec::new());
        }
        let len = if u64::try_from(size)? > entry.size {
            usize::try_from(entry.size)?
        } else {
            size
        };
        self.seek(SeekFrom::Start(start_offset + offset))?;
        let data = self.inner.get_reader_mut().read_exact_allocated(len)?;

        Ok(data)
    }
}
