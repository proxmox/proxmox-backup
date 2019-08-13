//! *pxar* format decoder for seekable files
//!
//! This module contain the code to decode *pxar* archive files.

use failure::*;

use super::format_definition::*;
use super::sequential_decoder::*;

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use std::ffi::OsString;


pub struct DirectoryEntry {
    start: u64,
    end: u64,
    pub filename: OsString,
    pub entry: PxarEntry,
}

// This one needs Read+Seek
pub struct Decoder<'a, R: Read + Seek, F: Fn(&Path) -> Result<(), Error>> {
    inner: SequentialDecoder<'a, R, F>,
    root_start: u64,
    root_end: u64,
}

const HEADER_SIZE: u64 = std::mem::size_of::<PxarHeader>() as u64;

impl <'a, R: Read + Seek, F: Fn(&Path) -> Result<(), Error>> Decoder<'a, R, F> {

    pub fn new(reader: &'a mut R, callback: F) -> Result<Self, Error> {

        let root_end = reader.seek(SeekFrom::End(0))?;

        Ok(Self {
            inner: SequentialDecoder::new(reader, super::flags::DEFAULT, callback),
            root_start: 0,
            root_end: root_end,
        })
    }

    pub fn root(&self) -> DirectoryEntry {
        DirectoryEntry {
            start: self.root_start,
            end: self.root_end,
            filename: OsString::new(), // Empty
            entry: PxarEntry {
                mode: 0,
                flags: 0,
                uid: 0,
                gid: 0,
                mtime: 0,
            }
        }
    }

    fn seek(&mut self, pos: SeekFrom) -> Result<u64, Error> {
        let pos = self.inner.get_reader_mut().seek(pos)?;
        Ok(pos)
    }

    pub fn restore(
        &mut self,
        dir: &DirectoryEntry,
        path: &Path,
    ) -> Result<(), Error> {
        let start = dir.start;

        self.seek(SeekFrom::Start(start))?;

        self.inner.restore(path, &Vec::new())?;

        Ok(())
    }

    fn read_directory_entry(&mut self, start: u64, end: u64) -> Result<DirectoryEntry, Error> {

        self.seek(SeekFrom::Start(start))?;

        let head: PxarHeader = self.inner.read_item()?;

        if head.htype != PXAR_FILENAME {
            bail!("wrong filename header type for object [{}..{}]", start, end);
        }

        let entry_start = start + head.size;

        let filename = self.inner.read_filename(head.size)?;

        let head: PxarHeader = self.inner.read_item()?;
        check_ca_header::<PxarEntry>(&head, PXAR_ENTRY)?;
        let entry: PxarEntry = self.inner.read_item()?;

        Ok(DirectoryEntry {
            start: entry_start,
            end: end,
            filename: filename,
            entry,
        })
    }

    pub fn list_dir(&mut self, dir: &DirectoryEntry) -> Result<Vec<DirectoryEntry>, Error> {

        const GOODBYE_ITEM_SIZE: u64 = std::mem::size_of::<PxarGoodbyeItem>() as u64;

        let start = dir.start;
        let end = dir.end;

        //println!("list_dir1: {} {}", start, end);

        if (end - start) < (HEADER_SIZE + GOODBYE_ITEM_SIZE) {
            bail!("detected short object [{}..{}]", start, end);
        }

        self.seek(SeekFrom::Start(end - GOODBYE_ITEM_SIZE))?;

        let item: PxarGoodbyeItem = self.inner.read_item()?;

        if item.hash != PXAR_GOODBYE_TAIL_MARKER {
            bail!("missing goodbye tail marker for object [{}..{}]", start, end);
        }

        let goodbye_table_size = item.size;
        if goodbye_table_size < (HEADER_SIZE + GOODBYE_ITEM_SIZE) {
            bail!("short goodbye table size for object [{}..{}]", start, end);

        }
        let goodbye_inner_size = goodbye_table_size - HEADER_SIZE - GOODBYE_ITEM_SIZE;
        if (goodbye_inner_size % GOODBYE_ITEM_SIZE) != 0 {
            bail!("wrong goodbye inner table size for entry [{}..{}]", start, end);
        }

        let goodbye_start = end - goodbye_table_size;

        if item.offset != (goodbye_start - start) {
            println!("DEBUG: {} {}", u64::from_le(item.offset), goodbye_start - start);
            bail!("wrong offset in goodbye tail marker for entry [{}..{}]", start, end);
        }

        self.seek(SeekFrom::Start(goodbye_start))?;
        let head: PxarHeader = self.inner.read_item()?;

        if head.htype != PXAR_GOODBYE {
            bail!("wrong goodbye table header type for entry [{}..{}]", start, end);
        }

        if head.size != goodbye_table_size {
            bail!("wrong goodbye table size for entry [{}..{}]", start, end);
        }

        let mut range_list = Vec::new();

        for i in 0..goodbye_inner_size/GOODBYE_ITEM_SIZE {
            let item: PxarGoodbyeItem = self.inner.read_item()?;

            if item.offset > (goodbye_start - start) {
                bail!("goodbye entry {} offset out of range [{}..{}] {} {} {}",
                      i, start, end, item.offset, goodbye_start, start);
            }
            let item_start = goodbye_start - item.offset;
            let item_end = item_start + item.size;
            if item_end > goodbye_start {
                bail!("goodbye entry {} end out of range [{}..{}]",
                      i, start, end);
            }

            range_list.push((item_start, item_end));
        }

        let mut result = vec![];

        for (item_start, item_end) in range_list {
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

            if ifmt == libc::S_IFDIR {
                self.print_filenames(output, prefix, item)?;
            } else if ifmt == libc::S_IFREG {
            } else if ifmt == libc::S_IFLNK {
            } else if ifmt == libc::S_IFBLK {
            } else if ifmt == libc::S_IFCHR {
            } else {
                bail!("unknown item mode/type for {:?}", prefix);
            }

            prefix.pop();
        }

        Ok(())
    }
}
