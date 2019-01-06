//! *catar* format decoder.
//!
//! This module contain the code to decode *catar* archive files.

use failure::*;

use super::format_definition::*;
use crate::tools;

use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;

use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::ffi::{OsStr, OsString};

pub struct CaDirectoryEntry {
    start: u64,
    end: u64,
    pub filename: OsString,
    pub entry: CaFormatEntry,
}

// This one needs Read+Seek (we may want one without Seek?)
pub struct CaTarDecoder<'a, R: Read + Seek> {
    reader: &'a mut R,
    root_start: u64,
    root_end: u64,
}

const HEADER_SIZE: u64 = std::mem::size_of::<CaFormatHeader>() as u64;

impl <'a, R: Read + Seek> CaTarDecoder<'a, R> {

    pub fn new(reader: &'a mut R) -> Result<Self, Error> {

        let root_end = reader.seek(SeekFrom::End(0))?;

        Ok(Self {
            reader: reader,
            root_start: 0,
            root_end: root_end,
        })
    }

    pub fn root(&self) -> CaDirectoryEntry {
        CaDirectoryEntry {
            start: self.root_start,
            end: self.root_end,
            filename: OsString::new(), // Empty
            entry: CaFormatEntry {
                feature_flags: 0,
                mode: 0,
                flags: 0,
                uid: 0,
                gid: 0,
                mtime: 0,
            }
        }
    }

    fn read_directory_entry(&mut self, start: u64, end: u64) -> Result<CaDirectoryEntry, Error> {

        self.reader.seek(SeekFrom::Start(start))?;
        let mut buffer = [0u8; HEADER_SIZE as usize];
        self.reader.read_exact(&mut buffer)?;
        let head = tools::map_struct::<CaFormatHeader>(&buffer)?;

        if u64::from_le(head.htype) != CA_FORMAT_FILENAME {
            bail!("wrong filename header type for object [{}..{}]", start, end);
        }

        let mut name_len = u64::from_le(head.size);

        let entry_start = start + name_len;

        if name_len < (HEADER_SIZE + 2) {
            bail!("filename size too short for object [{}..{}]", start, end);
        }
        name_len -= HEADER_SIZE;

        if name_len > ((libc::FILENAME_MAX as u64) + 1) {
            bail!("filename too long for object [{}..{}]", start, end);
        }

        let mut buffer = vec![0u8; name_len as usize];
        self.reader.read_exact(&mut buffer)?;

        // fixme: check nul termination
        let last_byte = buffer.pop().unwrap();
        if last_byte != 0u8 {
            bail!("filename entry not nul terminated, object [{}..{}]", start, end);

        }

        let filename = std::ffi::OsString::from_vec(buffer);

        let mut buffer = [0u8; HEADER_SIZE as usize];
        self.reader.read_exact(&mut buffer)?;
        let head = tools::map_struct::<CaFormatHeader>(&buffer)?;

        if u64::from_le(head.htype) != CA_FORMAT_ENTRY {
            bail!("wrong entry header type for object [{}..{}]", start, end);
        }

        const ENTRY_SIZE: u64 = std::mem::size_of::<CaFormatEntry>() as u64;

        let mut entry_len = u64::from_le(head.size);
        if entry_len != (HEADER_SIZE + ENTRY_SIZE) {
            bail!("wrong entry header size for object [{}..{}]", start, end);
        }
        entry_len -= HEADER_SIZE;

        let mut buffer = [0u8; ENTRY_SIZE as usize];
        self.reader.read_exact(&mut buffer)?;
        let entry = tools::map_struct::<CaFormatEntry>(&buffer)?;

        Ok(CaDirectoryEntry {
            start: entry_start,
            end: end,
            filename: filename,
            entry: CaFormatEntry {
                feature_flags: u64::from_le(entry.feature_flags),
                mode: u64::from_le(entry.mode),
                flags: u64::from_le(entry.flags),
                uid: u64::from_le(entry.uid),
                gid: u64::from_le(entry.gid),
                mtime: u64::from_le(entry.mtime),
            },
        })
    }

    pub fn list_dir(&mut self, dir: &CaDirectoryEntry) -> Result<Vec<CaDirectoryEntry>, Error> {

        const GOODBYE_ITEM_SIZE: u64 = std::mem::size_of::<CaFormatGoodbyeItem>() as u64;

        let start = dir.start;
        let end = dir.end;

        //println!("list_dir1: {} {}", start, end);

        if (end - start) < (HEADER_SIZE + GOODBYE_ITEM_SIZE) {
            bail!("detected short object [{}..{}]", start, end);
        }

        self.reader.seek(SeekFrom::Start(end - GOODBYE_ITEM_SIZE))?;
        let mut buffer = [0u8; GOODBYE_ITEM_SIZE as usize];
        self.reader.read_exact(&mut buffer)?;

        let item = tools::map_struct::<CaFormatGoodbyeItem>(&buffer)?;

        if u64::from_le(item.hash) != CA_FORMAT_GOODBYE_TAIL_MARKER {
            bail!("missing goodbye tail marker for object [{}..{}]", start, end);
        }

        let goodbye_table_size = u64::from_le(item.size);
        if goodbye_table_size < (HEADER_SIZE + GOODBYE_ITEM_SIZE) {
            bail!("short goodbye table size for object [{}..{}]", start, end);

        }
        let goodbye_inner_size = goodbye_table_size - HEADER_SIZE - GOODBYE_ITEM_SIZE;
        if (goodbye_inner_size % GOODBYE_ITEM_SIZE) != 0 {
            bail!("wrong goodbye inner table size for entry [{}..{}]", start, end);
        }

        let goodbye_start = end - goodbye_table_size;

        if u64::from_le(item.offset) != (goodbye_start - start) {
            println!("DEBUG: {} {}", u64::from_le(item.offset), goodbye_start - start);
            bail!("wrong offset in goodbye tail marker for entry [{}..{}]", start, end);
        }

        self.reader.seek(SeekFrom::Start(goodbye_start))?;
        let mut buffer = [0u8; HEADER_SIZE as usize];
        self.reader.read_exact(&mut buffer)?;
        let head = tools::map_struct::<CaFormatHeader>(&buffer)?;

        if u64::from_le(head.htype) != CA_FORMAT_GOODBYE {
            bail!("wrong goodbye table header type for entry [{}..{}]", start, end);
        }

        if u64::from_le(head.size) != goodbye_table_size {
            bail!("wrong goodbye table size for entry [{}..{}]", start, end);
        }

        let mut buffer = [0u8; GOODBYE_ITEM_SIZE as usize];

        let mut range_list = Vec::new();

        for i in 0..goodbye_inner_size/GOODBYE_ITEM_SIZE {
            self.reader.read_exact(&mut buffer)?;
            let item = tools::map_struct::<CaFormatGoodbyeItem>(&buffer)?;
            let item_offset = u64::from_le(item.offset);
            if item_offset > (goodbye_start - start) {
                bail!("goodbye entry {} offset out of range [{}..{}] {} {} {}",
                      i, start, end, item_offset, goodbye_start, start);
            }
            let item_start = goodbye_start - item_offset;
            let item_hash = u64::from_le(item.hash);
            let item_end = item_start + u64::from_le(item.size);
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
        dir: &CaDirectoryEntry,
    ) -> Result<(), Error> {

        let mut list = self.list_dir(dir)?;

        list.sort_unstable_by(|a, b| a.filename.cmp(&b.filename));

        for item in &list {

            prefix.push(item.filename.clone());

            let mode = item.entry.mode as u32;

            let osstr: &OsStr =  prefix.as_ref();
            output.write(osstr.as_bytes())?;
            output.write(b"\n")?;

            if (mode & libc::S_IFMT) == libc::S_IFDIR {
                self.print_filenames(output, prefix, item)?;
            } else if (mode & libc::S_IFMT) == libc::S_IFREG {
            } else if (mode & libc::S_IFMT) == libc::S_IFLNK {
            } else {
                bail!("unknown item mode/type for {:?}", prefix);
            }

            prefix.pop();
        }

        Ok(())
    }
}
