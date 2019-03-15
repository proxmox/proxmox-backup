//! *pxar* format decoder for seekable files
//!
//! This module contain the code to decode *pxar* archive files.

use failure::*;
use endian_trait::Endian;

use super::format_definition::*;
use super::sequential_decoder::*;
use crate::tools;

use std::io::{Read, Write, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use std::os::unix::io::AsRawFd;
use std::os::unix::io::RawFd;
use std::os::unix::io::FromRawFd;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::ffi::{OsStr, OsString};

use nix::fcntl::OFlag;
use nix::sys::stat::Mode;
use nix::errno::Errno;
use nix::NixPath;

pub struct CaDirectoryEntry {
    start: u64,
    end: u64,
    pub filename: OsString,
    pub entry: CaFormatEntry,
}

// This one needs Read+Seek
pub struct Decoder<'a, R: Read + Seek> {
    inner: SequentialDecoder<'a, R>,
    root_start: u64,
    root_end: u64,
}

const HEADER_SIZE: u64 = std::mem::size_of::<CaFormatHeader>() as u64;

impl <'a, R: Read + Seek> Decoder<'a, R> {

    pub fn new(reader: &'a mut R) -> Result<Self, Error> {

        let root_end = reader.seek(SeekFrom::End(0))?;

        Ok(Self {
            inner: SequentialDecoder::new(reader),
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

    fn seek(&mut self, pos: SeekFrom) -> Result<u64, Error> {
        let pos = self.inner.get_reader_mut().seek(pos)?;
        Ok(pos)
    }

    pub fn restore<F>(
        &mut self,
        dir: &CaDirectoryEntry,
        path: &Path,
        callback: F,
    ) -> Result<(), Error>
        where F: Fn(&Path) -> Result<(), Error>
    {
        let start = dir.start;

        self.seek(SeekFrom::Start(start))?;

        self.inner.restore(path, &callback)?;

        Ok(())
    }

    fn read_directory_entry(&mut self, start: u64, end: u64) -> Result<CaDirectoryEntry, Error> {

        self.seek(SeekFrom::Start(start))?;

        let head: CaFormatHeader = self.inner.read_item()?;

        if head.htype != CA_FORMAT_FILENAME {
            bail!("wrong filename header type for object [{}..{}]", start, end);
        }

        let entry_start = start + head.size;

        let filename = self.inner.read_filename(head.size)?;

        let head: CaFormatHeader = self.inner.read_item()?;
        check_ca_header::<CaFormatEntry>(&head, CA_FORMAT_ENTRY)?;
        let entry: CaFormatEntry = self.inner.read_item()?;

        Ok(CaDirectoryEntry {
            start: entry_start,
            end: end,
            filename: filename,
            entry,
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

        self.seek(SeekFrom::Start(end - GOODBYE_ITEM_SIZE))?;

        let item: CaFormatGoodbyeItem = self.inner.read_item()?;

        if item.hash != CA_FORMAT_GOODBYE_TAIL_MARKER {
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
        let head: CaFormatHeader = self.inner.read_item()?;

        if head.htype != CA_FORMAT_GOODBYE {
            bail!("wrong goodbye table header type for entry [{}..{}]", start, end);
        }

        if head.size != goodbye_table_size {
            bail!("wrong goodbye table size for entry [{}..{}]", start, end);
        }

        let mut range_list = Vec::new();

        for i in 0..goodbye_inner_size/GOODBYE_ITEM_SIZE {
            let item: CaFormatGoodbyeItem = self.inner.read_item()?;

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
        dir: &CaDirectoryEntry,
    ) -> Result<(), Error> {

        let mut list = self.list_dir(dir)?;

        list.sort_unstable_by(|a, b| a.filename.cmp(&b.filename));

        for item in &list {

            prefix.push(item.filename.clone());

            let mode = item.entry.mode as u32;

            let ifmt = mode & libc::S_IFMT;

            let osstr: &OsStr =  prefix.as_ref();
            output.write(osstr.as_bytes())?;
            output.write(b"\n")?;

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

fn file_openat(parent: RawFd, filename: &OsStr, flags: OFlag, mode: Mode) -> Result<std::fs::File, Error> {

    let fd = filename.with_nix_path(|cstr| {
        nix::fcntl::openat(parent, cstr.as_ref(), flags, mode)
    })??;

    let file = unsafe { std::fs::File::from_raw_fd(fd) };

    Ok(file)
}

fn dir_mkdirat(parent: RawFd, filename: &OsStr) -> Result<nix::dir::Dir, Error> {

    // call mkdirat first
    let res = filename.with_nix_path(|cstr| unsafe {
        libc::mkdirat(parent, cstr.as_ptr(), libc::S_IRWXU)
    })?;
    Errno::result(res)?;

    let dir = nix::dir::Dir::openat(parent, filename, OFlag::O_DIRECTORY,  Mode::empty())?;

    Ok(dir)
}

fn symlinkat(target: &Path, parent: RawFd, linkname: &OsStr) -> Result<(), Error> {

    target.with_nix_path(|target| {
        linkname.with_nix_path(|linkname| {
            let res = unsafe { libc::symlinkat(target.as_ptr(), parent, linkname.as_ptr()) };
            Errno::result(res)?;
            Ok(())
        })?
    })?
}

fn nsec_to_update_timespec(mtime_nsec: u64) -> [libc::timespec; 2] {

    // restore mtime
    const UTIME_OMIT: i64 = ((1 << 30) - 2);
    const NANOS_PER_SEC: i64 = 1_000_000_000;

    let sec = (mtime_nsec as i64) / NANOS_PER_SEC;
    let nsec = (mtime_nsec as i64) % NANOS_PER_SEC;

    let times: [libc::timespec; 2] = [
        libc::timespec { tv_sec: 0, tv_nsec: UTIME_OMIT },
        libc::timespec { tv_sec: sec, tv_nsec: nsec },
    ];

    times
}
