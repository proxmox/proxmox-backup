//! *pxar* format decoder.
//!
//! This module contain the code to decode *pxar* archive files.

use failure::*;
use endian_trait::Endian;

use super::format_definition::*;

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use std::os::unix::io::AsRawFd;
use std::os::unix::io::RawFd;
use std::os::unix::io::FromRawFd;
use std::os::unix::ffi::{OsStringExt};
use std::ffi::{OsStr, OsString};

use nix::fcntl::OFlag;
use nix::sys::stat::Mode;
use nix::errno::Errno;
use nix::NixPath;

// This one need Read, but works without Seek
pub struct PxarDecoder<'a, R: Read> {
    reader: &'a mut R,
    skip_buffer: Vec<u8>,
}

const HEADER_SIZE: u64 = std::mem::size_of::<CaFormatHeader>() as u64;

impl <'a, R: Read> PxarDecoder<'a, R> {

    pub fn new(reader: &'a mut R) -> Self {
        let mut skip_buffer = vec![0u8; 64*1024];
        Self { reader, skip_buffer }
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

        if (buffer.len() == 1 && buffer[0] == b'.') || (buffer.len() == 2 && buffer[0] == b'.' && buffer[1] == b'.') {
            bail!("found invalid filename with slashes.");
        }

        if buffer.iter().find(|b| (**b == b'/')).is_some() {
            bail!("found invalid filename with slashes.");
        }

        let name = std::ffi::OsString::from_vec(buffer);
        if name.is_empty() {
            bail!("found empty filename.");
        }

        Ok(name)
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

    fn skip_bytes(&mut self, count: usize) -> Result<(), Error> {
        let mut done = 0;
        while done < count  {
            let todo = count - done;
            let n = if todo > self.skip_buffer.len() { self.skip_buffer.len() } else { todo };
            let data = &mut self.skip_buffer[..n];
            self.reader.read_exact(data)?;
            done += n;
        }
        Ok(())
    }

    pub fn restore<F>(
        &mut self,
        path: &Path, // used for error reporting
        callback: &F,
    ) -> Result<(), Error>
        where F: Fn(&Path) -> Result<(), Error>
    {

        let _ = std::fs::create_dir(path);

        let dir = match nix::dir::Dir::open(path, nix::fcntl::OFlag::O_DIRECTORY,  nix::sys::stat::Mode::empty()) {
            Ok(dir) => dir,
            Err(err) => bail!("unable to open target directory {:?} - {}", path, err),
        };

        self.restore_sequential(&mut path.to_owned(), &OsString::new(), &dir, callback)
    }

    fn restore_sequential<F>(
        &mut self,
        path: &mut PathBuf, // used for error reporting
        filename: &OsStr,  // repeats path last component
        parent: &nix::dir::Dir,
        callback: &F,
    ) -> Result<(), Error>
        where F: Fn(&Path) -> Result<(), Error>
    {

        let parent_fd = parent.as_raw_fd();

        // read ENTRY first
        let head: CaFormatHeader = self.read_item()?;
        check_ca_header::<CaFormatEntry>(&head, CA_FORMAT_ENTRY)?;
        let entry: CaFormatEntry = self.read_item()?;

        let mode = entry.mode as u32; //fixme: upper 32bits?

        let ifmt = mode & libc::S_IFMT;

        if ifmt == libc::S_IFDIR {
            let dir;
            if filename.is_empty() {
                dir = nix::dir::Dir::openat(parent_fd, ".", OFlag::O_DIRECTORY,  Mode::empty())?;
             } else {
                dir = match dir_mkdirat(parent_fd, filename, true) {
                    Ok(dir) => dir,
                    Err(err) => bail!("unable to open directory {:?} - {}", path, err),
                };
            }

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

            self.skip_bytes((head.size - HEADER_SIZE) as usize)?;

            self.restore_mode(&entry, dir.as_raw_fd())?;
            self.restore_mtime(&entry, dir.as_raw_fd())?;
            self.restore_ugid(&entry, dir.as_raw_fd())?;

            return Ok(());
        }

        if filename.is_empty() {
            bail!("got empty file name at {:?}", path)
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

    /// List/Dump archive content.
    ///
    /// Simply print the list of contained files. This dumps archive
    /// format details when the verbose flag is set (useful for debug).
    pub fn dump_entry<W: std::io::Write>(
        &mut self,
        path: &mut PathBuf,
        verbose: bool,
        output: &mut W,
    ) -> Result<(), Error> {

        let print_head = |head: &CaFormatHeader| {
            println!("Type: {:016x}", head.htype);
            println!("Size: {}", head.size);
        };

        let head: CaFormatHeader = self.read_item()?;
        if verbose {
            println!("Path: {:?}", path);
            print_head(&head);
        } else {
            println!("{:?}", path);
        }

        check_ca_header::<CaFormatEntry>(&head, CA_FORMAT_ENTRY)?;
        let entry: CaFormatEntry = self.read_item()?;

        if verbose {
            println!("Mode: {:08x} {:08x}", entry.mode, (entry.mode as u32) & libc::S_IFDIR);
        }
        // fixme: dump attributes (ACLs, ...)

        let ifmt = (entry.mode as u32) & libc::S_IFMT;

        if ifmt == libc::S_IFDIR {

            let mut entry_count = 0;

            loop {
                let head: CaFormatHeader = self.read_item()?;
                if verbose {
                    print_head(&head);
                }
                match head.htype {

                    CA_FORMAT_FILENAME =>  {
                        let name = self.read_filename(head.size)?;
                        if verbose { println!("Name: {:?}", name); }
                        entry_count += 1;
                        path.push(&name);
                        self.dump_entry(path, verbose, output)?;
                        path.pop();
                    }
                    CA_FORMAT_GOODBYE => {
                        let table_size = (head.size - HEADER_SIZE) as usize;
                        if verbose {
                            println!("Goodbye: {:?}", path);
                            self.dump_goodby_entries(entry_count, table_size)?;
                        } else {
                            self.skip_bytes(table_size);
                        }
                        break;
                    }
                    _ => {
                        panic!("got unexpected header type inside directory");
                    }
                }
            }
        } else {

            let head: CaFormatHeader = self.read_item()?;
            if verbose {
                print_head(&head);
            }

            match head.htype {

                CA_FORMAT_SYMLINK => {
                    let target = self.read_symlink(head.size)?;
                    if verbose {
                        println!("Symlink: {:?}", target);
                    }
                }
                CA_FORMAT_DEVICE => {
                    let device: CaFormatDevice = self.read_item()?;
                    if verbose {
                        println!("Device: {}, {}", device.major, device.minor);
                    }
                }
                CA_FORMAT_PAYLOAD => {
                    let payload_size = (head.size - HEADER_SIZE) as usize;
                    if verbose {
                        println!("Payload: {}", payload_size);
                    }
                    self.skip_bytes(payload_size)?;
                }
                _ => {
                    panic!("got unexpected header type inside non-directory");
                }
            }
        }

        Ok(())
    }

    fn dump_goodby_entries(
        &mut self,
        entry_count: usize,
        table_size: usize,
    ) -> Result<(), Error> {

        let item_size = std::mem::size_of::<CaFormatGoodbyeItem>();
        if table_size < item_size {
            bail!("Goodbye table to small ({} < {})", table_size, item_size);
        }
        if (table_size % item_size) != 0 {
            bail!("Goodbye table with strange size ({})", table_size);
        }

        let entries = (table_size / item_size);

        if entry_count != (entries - 1) {
            bail!("Goodbye table with wrong entry count ({} != {})", entry_count, entries - 1);
        }

        let mut count = 0;

        loop {
            let item: CaFormatGoodbyeItem = self.read_item()?;
            count += 1;
            if item.hash == CA_FORMAT_GOODBYE_TAIL_MARKER {
                if count != entries {
                    bail!("unexpected goodbye tail marker");
                }
                println!("Goodby tail mark.");
                break;
            }
            println!("Goodby item: offset {}, size {}, hash {:016x}", item.offset, item.size, item.hash);
            if count >= (table_size / item_size) {
                bail!("too many goodbye items (no tail marker)");
            }
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

fn dir_mkdirat(parent: RawFd, filename: &OsStr, create_new: bool) -> Result<nix::dir::Dir, nix::Error> {

    // call mkdirat first
    let res = filename.with_nix_path(|cstr| unsafe {
        libc::mkdirat(parent, cstr.as_ptr(), libc::S_IRWXU)
    })?;

    match Errno::result(res) {
        Ok(_) => {},
        Err(err) => {
            if err == nix::Error::Sys(nix::errno::Errno::EEXIST) {
                if create_new { return Err(err); }
            } else {
                return Err(err);
            }
        }
    }

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
