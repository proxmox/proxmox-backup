//! *pxar* format decoder.
//!
//! This module contain the code to decode *pxar* archive files.

use failure::*;
use endian_trait::Endian;

use super::format_definition::*;
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

// This one needs Read+Seek (we may want one without Seek?)
pub struct PxarDecoder<'a, R: Read + Seek> {
    reader: &'a mut R,
    root_start: u64,
    root_end: u64,
}

const HEADER_SIZE: u64 = std::mem::size_of::<CaFormatHeader>() as u64;

impl <'a, R: Read + Seek> PxarDecoder<'a, R> {

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

    fn read_item<T: Endian>(&mut self) -> Result<T, Error> {

        let mut result: T = unsafe { std::mem::uninitialized() };

        let buffer = unsafe { std::slice::from_raw_parts_mut(
            &mut result as *mut T as *mut u8,
            std::mem::size_of::<T>()
        )};

        self.reader.read_exact(buffer)?;

        Ok(result.from_le())
    }

    fn read_symlink(&mut self, size: u64) -> Result<PathBuf, Error> {
        if size < (HEADER_SIZE + 2) {
             bail!("dectected short symlink target.");
        }
        let target_len = size - HEADER_SIZE;

        if target_len > (libc::PATH_MAX as u64) {
            bail!("symlink target too long ({}).", target_len);
        }

        let mut buffer = vec![0u8; target_len as usize];
        self.reader.read_exact(&mut buffer)?;

        let last_byte = buffer.pop().unwrap();
        if last_byte != 0u8 {
            bail!("symlink target not nul terminated.");
        }

        Ok(PathBuf::from(std::ffi::OsString::from_vec(buffer)))
    }

    fn read_filename(&mut self, size: u64) -> Result<OsString, Error> {
        if size < (HEADER_SIZE + 2) {
            bail!("dectected short filename");
        }
        let name_len = size - HEADER_SIZE;

        if name_len > ((libc::FILENAME_MAX as u64) + 1) {
            bail!("filename too long ({}).", name_len);
        }

        let mut buffer = vec![0u8; name_len as usize];
        self.reader.read_exact(&mut buffer)?;

        let last_byte = buffer.pop().unwrap();
        if last_byte != 0u8 {
            bail!("filename entry not nul terminated.");
        }

        if buffer.iter().find(|b| (**b == b'/')).is_some() {
            bail!("found invalid filename with slashes.");
        }

        Ok(std::ffi::OsString::from_vec(buffer))
    }

    pub fn restore<F: Fn(&Path) -> Result<(), Error>>(
        &mut self,
        dir: &CaDirectoryEntry,
        callback: F,
    ) -> Result<(), Error> {

        let start = dir.start;

        self.reader.seek(SeekFrom::Start(start))?;

        let base = ".";

        let mut path = PathBuf::from(base);

        let dir = match nix::dir::Dir::open(&path, nix::fcntl::OFlag::O_DIRECTORY,  nix::sys::stat::Mode::empty()) {
            Ok(dir) => dir,
            Err(err) => bail!("unable to open base directory - {}", err),
        };

        let restore_dir = "restoretest";
        path.push(restore_dir);

        self.restore_sequential(&mut path, &OsString::from(restore_dir), &dir, &callback)?;

        Ok(())
    }

    fn restore_attributes(&mut self, _entry: &CaFormatEntry) -> Result<CaFormatHeader, Error> {

        loop {
            let head: CaFormatHeader = self.read_item()?;
            match head.htype {
                // fimxe: impl ...
                _ => return Ok(head),
            }
        }
    }

    fn restore_mode(&mut self, entry: &CaFormatEntry, fd: RawFd) -> Result<(), Error> {

        let mode = Mode::from_bits_truncate((entry.mode as u32) & 0o7777);

        nix::sys::stat::fchmod(fd, mode)?;

        Ok(())
    }

    fn restore_mode_at(&mut self, entry: &CaFormatEntry, dirfd: RawFd, filename: &OsStr) -> Result<(), Error> {

        let mode = Mode::from_bits_truncate((entry.mode as u32) & 0o7777);

        // NOTE: we want :FchmodatFlags::NoFollowSymlink, but fchmodat does not support that
        // on linux (see man fchmodat). Fortunately, we can simply avoid calling this on symlinks.
        nix::sys::stat::fchmodat(Some(dirfd), filename, mode, nix::sys::stat::FchmodatFlags::FollowSymlink)?;

        Ok(())
    }

    fn restore_ugid(&mut self, entry: &CaFormatEntry, fd: RawFd) -> Result<(), Error> {

        let uid = entry.uid as u32;
        let gid = entry.gid as u32;

        let res = unsafe { libc::fchown(fd, uid, gid) };
        Errno::result(res)?;

        Ok(())
    }

    fn restore_ugid_at(&mut self, entry: &CaFormatEntry, dirfd: RawFd,  filename: &OsStr) -> Result<(), Error> {

        let uid = entry.uid as u32;
        let gid = entry.gid as u32;

        let res = filename.with_nix_path(|cstr| unsafe {
            libc::fchownat(dirfd, cstr.as_ptr(), uid, gid, libc::AT_SYMLINK_NOFOLLOW)
        })?;
        Errno::result(res)?;

        Ok(())
    }

    fn restore_mtime(&mut self, entry: &CaFormatEntry, fd: RawFd) -> Result<(), Error> {

        let times = nsec_to_update_timespec(entry.mtime);

        let res = unsafe { libc::futimens(fd, &times[0]) };
        Errno::result(res)?;

        Ok(())
    }

    fn restore_mtime_at(&mut self, entry: &CaFormatEntry, dirfd: RawFd, filename: &OsStr) -> Result<(), Error> {

        let times = nsec_to_update_timespec(entry.mtime);

        let res =  filename.with_nix_path(|cstr| unsafe {
            libc::utimensat(dirfd, cstr.as_ptr(), &times[0],  libc::AT_SYMLINK_NOFOLLOW)
        })?;
        Errno::result(res)?;

        Ok(())
    }

    fn restore_device_at(&mut self, entry: &CaFormatEntry, dirfd: RawFd, filename: &OsStr, device: &CaFormatDevice) -> Result<(), Error> {

        let rdev = nix::sys::stat::makedev(device.major, device.minor);
        let mode = ((entry.mode as u32) & libc::S_IFMT) | 0o0600;
        let res =  filename.with_nix_path(|cstr| unsafe {
            libc::mknodat(dirfd, cstr.as_ptr(), mode, rdev)
        })?;
        Errno::result(res)?;

        Ok(())
    }

    fn restore_socket_at(&mut self, dirfd: RawFd, filename: &OsStr) -> Result<(), Error> {

        let mode = libc::S_IFSOCK | 0o0600;
        let res =  filename.with_nix_path(|cstr| unsafe {
            libc::mknodat(dirfd, cstr.as_ptr(), mode, 0)
        })?;
        Errno::result(res)?;

        Ok(())
    }

    fn restore_fifo_at(&mut self, dirfd: RawFd, filename: &OsStr) -> Result<(), Error> {

        let mode = libc::S_IFIFO | 0o0600;
        let res =  filename.with_nix_path(|cstr| unsafe {
            libc::mkfifoat(dirfd, cstr.as_ptr(), mode)
        })?;
        Errno::result(res)?;

        Ok(())
    }

    pub fn restore_sequential<F: Fn(&Path) -> Result<(), Error>>(
        &mut self,
        path: &mut PathBuf, // user for error reporting
        filename: &OsStr,  // repeats path last component
        parent: &nix::dir::Dir,
        callback: &F,
    ) -> Result<(), Error> {

        let parent_fd = parent.as_raw_fd();

        // read ENTRY first
        let head: CaFormatHeader = self.read_item()?;
        check_ca_header::<CaFormatEntry>(&head, CA_FORMAT_ENTRY)?;
        let entry: CaFormatEntry = self.read_item()?;

        let mode = entry.mode as u32; //fixme: upper 32bits?

        let ifmt = mode & libc::S_IFMT;

        if ifmt == libc::S_IFDIR {
            let dir = match dir_mkdirat(parent_fd, filename) {
                Ok(dir) => dir,
                Err(err) => bail!("unable to open directory {:?} - {}", path, err),
            };

            let mut head = self.restore_attributes(&entry)?;

            while head.htype == CA_FORMAT_FILENAME {
                let name = self.read_filename(head.size)?;
                path.push(&name);
                println!("NAME: {:?}", path);
                self.restore_sequential(path, &name, &dir, callback)?;
                path.pop();

                head = self.read_item()?;
            }

            if head.htype != CA_FORMAT_GOODBYE {
                bail!("got unknown header type inside directory entry {:016x}", head.htype);
            }

            println!("Skip Goodbye");
            if head.size < HEADER_SIZE { bail!("detected short goodbye table"); }

            // self.reader.seek(SeekFrom::Current((head.size - HEADER_SIZE) as i64))?;
            let mut done = 0;
            let skip = (head.size - HEADER_SIZE) as usize;
            let mut skip_buffer = vec![0u8; 64*1024];
            while done < skip  {
                let todo = skip - done;
                let n = if todo > skip_buffer.len() { skip_buffer.len() } else { todo };
                let data = &mut skip_buffer[..n];
                self.reader.read_exact(data)?;
                done += n;
            }


            self.restore_mode(&entry, dir.as_raw_fd())?;
            self.restore_mtime(&entry, dir.as_raw_fd())?;
            self.restore_ugid(&entry, dir.as_raw_fd())?;

            return Ok(());
        }

        if ifmt == libc::S_IFLNK {
            // fixme: create symlink
            //fixme: restore permission, acls, xattr, ...

            let head: CaFormatHeader = self.read_item()?;
            match head.htype {
                CA_FORMAT_SYMLINK => {
                    let target = self.read_symlink(head.size)?;
                    println!("TARGET: {:?}", target);
                    if let Err(err) = symlinkat(&target, parent_fd, filename) {
                        bail!("create symlink {:?} failed - {}", path, err);
                    }
                }
                 _ => {
                     bail!("got unknown header type inside symlink entry {:016x}", head.htype);
                 }
            }

            // self.restore_mode_at(&entry, parent_fd, filename)?; //not supported on symlinks
            self.restore_ugid_at(&entry, parent_fd, filename)?;
            self.restore_mtime_at(&entry, parent_fd, filename)?;

            return Ok(());
        }

        if ifmt == libc::S_IFSOCK  {

            self.restore_socket_at(parent_fd, filename)?;

            self.restore_mode_at(&entry, parent_fd, filename)?;
            self.restore_ugid_at(&entry, parent_fd, filename)?;
            self.restore_mtime_at(&entry, parent_fd, filename)?;

            return Ok(());
        }

        if ifmt == libc::S_IFIFO  {

            self.restore_fifo_at(parent_fd, filename)?;

            self.restore_mode_at(&entry, parent_fd, filename)?;
            self.restore_ugid_at(&entry, parent_fd, filename)?;
            self.restore_mtime_at(&entry, parent_fd, filename)?;

            return Ok(());
        }

        if (ifmt == libc::S_IFBLK) || (ifmt == libc::S_IFCHR)  {

            let head: CaFormatHeader = self.read_item()?;
            match head.htype {
                CA_FORMAT_DEVICE => {
                    let device: CaFormatDevice = self.read_item()?;
                    self.restore_device_at(&entry, parent_fd, filename, &device)?;
                }
                _ => {
                    bail!("got unknown header type inside device entry {:016x}", head.htype);
                }
            }

            self.restore_mode_at(&entry, parent_fd, filename)?;
            self.restore_ugid_at(&entry, parent_fd, filename)?;
            self.restore_mtime_at(&entry, parent_fd, filename)?;

            return Ok(());
        }

        if ifmt == libc::S_IFREG {

            let mut read_buffer: [u8; 64*1024] = unsafe { std::mem::uninitialized() };

            let flags = OFlag::O_CREAT|OFlag::O_WRONLY|OFlag::O_EXCL;
            let open_mode =  Mode::from_bits_truncate(0o0600 | mode);

            let mut file = match file_openat(parent_fd, filename, flags, open_mode) {
                Ok(file) => file,
                Err(err) => bail!("open file {:?} failed - {}", path, err),
            };

            let head = self.restore_attributes(&entry)?;

            if head.htype != CA_FORMAT_PAYLOAD {
                  bail!("got unknown header type for file entry {:016x}", head.htype);
            }

            if head.size < HEADER_SIZE {
                bail!("detected short payload");
            }
            let need = (head.size - HEADER_SIZE) as usize;
            //self.reader.seek(SeekFrom::Current(need as i64))?;

            let mut done = 0;
            while done < need  {
                let todo = need - done;
                let n = if todo > read_buffer.len() { read_buffer.len() } else { todo };
                let data = &mut read_buffer[..n];
                self.reader.read_exact(data)?;
                file.write_all(data)?;
                done += n;
            }

            self.restore_mode(&entry, file.as_raw_fd())?;
            self.restore_mtime(&entry, file.as_raw_fd())?;
            self.restore_ugid(&entry, file.as_raw_fd())?;

            return Ok(());
        }

        Ok(())
    }

    fn read_directory_entry(&mut self, start: u64, end: u64) -> Result<CaDirectoryEntry, Error> {

        self.reader.seek(SeekFrom::Start(start))?;
        let mut buffer = [0u8; HEADER_SIZE as usize];
        self.reader.read_exact(&mut buffer)?;
        let head = tools::map_struct::<CaFormatHeader>(&buffer)?;

        if u64::from_le(head.htype) != CA_FORMAT_FILENAME {
            bail!("wrong filename header type for object [{}..{}]", start, end);
        }

        let name_len = u64::from_le(head.size);

        let entry_start = start + name_len;

        let filename = self.read_filename(name_len)?;

        let head: CaFormatHeader = self.read_item()?;
        check_ca_header::<CaFormatEntry>(&head, CA_FORMAT_ENTRY)?;
        let entry: CaFormatEntry = self.read_item()?;

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
            let _item_hash = u64::from_le(item.hash);
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
