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

use crate::tools::io::ops::*;
use crate::tools::vec;
use crate::tools::acl;
use crate::tools::xattr;

// This one need Read, but works without Seek
pub struct SequentialDecoder<'a, R: Read> {
    reader: &'a mut R,
    feature_flags: u64,
    skip_buffer: Vec<u8>,
}

const HEADER_SIZE: u64 = std::mem::size_of::<CaFormatHeader>() as u64;

impl <'a, R: Read> SequentialDecoder<'a, R> {

    pub fn new(reader: &'a mut R, feature_flags: u64) -> Self {
        let skip_buffer = vec::undefined(64*1024);

        Self {
            reader,
            feature_flags,
            skip_buffer
        }
    }

    pub (crate) fn get_reader_mut(&mut self) -> & mut R {
        self.reader
    }

    pub (crate) fn read_item<T: Endian>(&mut self) -> Result<T, Error> {

        let mut result: T = unsafe { std::mem::uninitialized() };

        let buffer = unsafe { std::slice::from_raw_parts_mut(
            &mut result as *mut T as *mut u8,
            std::mem::size_of::<T>()
        )};

        self.reader.read_exact(buffer)?;

        Ok(result.from_le())
    }

    fn read_link(&mut self, size: u64) -> Result<PathBuf, Error> {
        if size < (HEADER_SIZE + 2) {
             bail!("dectected short link target.");
        }
        let target_len = size - HEADER_SIZE;

        if target_len > (libc::PATH_MAX as u64) {
            bail!("link target too long ({}).", target_len);
        }

        let mut buffer = self.reader.read_exact_allocated(target_len as usize)?;

        let last_byte = buffer.pop().unwrap();
        if last_byte != 0u8 {
            bail!("link target not nul terminated.");
        }

        Ok(PathBuf::from(std::ffi::OsString::from_vec(buffer)))
    }

    fn read_hardlink(&mut self, size: u64) -> Result<(PathBuf, u64), Error> {
        if size < (HEADER_SIZE + 8 + 2) {
            bail!("dectected short hardlink header.");
        }
        let offset: u64 = self.read_item()?;
        let target = self.read_link(size - 8)?;

        for c in target.components() {
            match c {
                std::path::Component::Normal(_) => { /* OK */  },
                _ => {
                    bail!("hardlink target contains invalid component {:?}", c);
                }
            }
        }

        Ok((target, offset))
    }

    pub (crate) fn read_filename(&mut self, size: u64) -> Result<OsString, Error> {
        if size < (HEADER_SIZE + 2) {
            bail!("dectected short filename");
        }
        let name_len = size - HEADER_SIZE;

        if name_len > ((libc::FILENAME_MAX as u64) + 1) {
            bail!("filename too long ({}).", name_len);
        }

        let mut buffer = self.reader.read_exact_allocated(name_len as usize)?;

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

    fn has_features(&self, feature_flags: u64) -> bool {
        (self.feature_flags & feature_flags) == feature_flags
    }

    fn read_xattr(&mut self, size: usize) -> Result<CaFormatXAttr, Error> {
        let buffer = self.reader.read_exact_allocated(size)?;

        let separator = buffer.iter().position(|c| *c == b'\0')
            .ok_or_else(|| format_err!("no value found in xattr"))?;

        let (name, value) = buffer.split_at(separator);
        if !xattr::is_valid_xattr_name(name) ||
            xattr::is_security_capability(name)
        {
            bail!("incorrect xattr name - {}.", String::from_utf8_lossy(name));
        }

        Ok(CaFormatXAttr {
            name: name.to_vec(),
            value: value[1..].to_vec(),
        })
    }

    fn read_fcaps(&mut self, size: usize) -> Result<CaFormatFCaps, Error> {
        let buffer = self.reader.read_exact_allocated(size)?;

        Ok(CaFormatFCaps { data: buffer })
    }

    fn restore_attributes(&mut self, entry: &CaFormatEntry, fd: RawFd) -> Result<CaFormatHeader, Error> {
        let mut xattrs = Vec::new();
        let mut fcaps = None;

        let mut acl_user = Vec::new();
        let mut acl_group = Vec::new();
        let mut acl_group_obj = None;

        let mut acl_default = None;
        let mut acl_default_user = Vec::new();
        let mut acl_default_group = Vec::new();

        let mut head: CaFormatHeader = self.read_item()?;
        let mut size = (head.size - HEADER_SIZE) as usize;
        loop {
            match head.htype {
                CA_FORMAT_XATTR => {
                    if self.has_features(CA_FORMAT_WITH_XATTRS) {
                        xattrs.push(self.read_xattr(size)?);
                    } else {
                        self.skip_bytes(size)?;
                    }
                },
                CA_FORMAT_FCAPS => {
                    if self.has_features(CA_FORMAT_WITH_FCAPS) {
                        fcaps = Some(self.read_fcaps(size)?);
                    } else {
                        self.skip_bytes(size)?;
                    }
                },
                CA_FORMAT_ACL_USER => {
                    if self.has_features(CA_FORMAT_WITH_ACL) {
                        acl_user.push(self.read_item::<CaFormatACLUser>()?);
                    } else {
                        self.skip_bytes(size)?;
                    }
                },
                CA_FORMAT_ACL_GROUP => {
                    if self.has_features(CA_FORMAT_WITH_ACL) {
                        acl_group.push(self.read_item::<CaFormatACLGroup>()?);
                    } else {
                        self.skip_bytes(size)?;
                    }
                },
                CA_FORMAT_ACL_GROUP_OBJ => {
                    if self.has_features(CA_FORMAT_WITH_ACL) {
                        acl_group_obj = Some(self.read_item::<CaFormatACLGroupObj>()?);
                    } else {
                        self.skip_bytes(size)?;
                    }
                },
                CA_FORMAT_ACL_DEFAULT => {
                    if self.has_features(CA_FORMAT_WITH_ACL) {
                        acl_default = Some(self.read_item::<CaFormatACLDefault>()?);
                    } else {
                        self.skip_bytes(size)?;
                    }
                },
                CA_FORMAT_ACL_DEFAULT_USER => {
                    if self.has_features(CA_FORMAT_WITH_ACL) {
                        acl_default_user.push(self.read_item::<CaFormatACLUser>()?);
                    } else {
                        self.skip_bytes(size)?;
                    }
                },
                CA_FORMAT_ACL_DEFAULT_GROUP => {
                    if self.has_features(CA_FORMAT_WITH_ACL) {
                        acl_default_group.push(self.read_item::<CaFormatACLGroup>()?);
                    } else {
                        self.skip_bytes(size)?;
                    }
                },
                _ => break,
            }
            head = self.read_item()?;
            size = (head.size - HEADER_SIZE) as usize;
        }
        self.restore_xattrs_fcaps_fd(fd, xattrs, fcaps)?;

        let mut acl = acl::ACL::init(5)?;
        acl.add_entry_full(acl::ACL_USER_OBJ, None, mode_user_to_acl_permissions(entry.mode))?;
        acl.add_entry_full(acl::ACL_OTHER, None, mode_other_to_acl_permissions(entry.mode))?;
        match acl_group_obj {
            Some(group_obj) => {
                acl.add_entry_full(acl::ACL_MASK, None, mode_group_to_acl_permissions(entry.mode))?;
                acl.add_entry_full(acl::ACL_GROUP_OBJ, None, group_obj.permissions)?;
            },
            None => {
                acl.add_entry_full(acl::ACL_GROUP_OBJ, None, mode_group_to_acl_permissions(entry.mode))?;
            },
        }
        for user in acl_user {
            acl.add_entry_full(acl::ACL_USER, Some(user.uid), user.permissions)?;
        }
        for group in acl_group {
            acl.add_entry_full(acl::ACL_GROUP, Some(group.gid), group.permissions)?;
        }
        let proc_path = Path::new("/proc/self/fd/").join(fd.to_string());
        if !acl.is_valid() {
            bail!("Error while restoring ACL - ACL invalid");
        }
        acl.set_file(&proc_path, acl::ACL_TYPE_ACCESS)?;

        if let Some(default) = acl_default {
            let mut acl = acl::ACL::init(5)?;
            acl.add_entry_full(acl::ACL_USER_OBJ, None, default.user_obj_permissions)?;
            acl.add_entry_full(acl::ACL_GROUP_OBJ, None, default.group_obj_permissions)?;
            acl.add_entry_full(acl::ACL_OTHER, None, default.other_permissions)?;
            if default.mask_permissions != std::u64::MAX {
                acl.add_entry_full(acl::ACL_MASK, None, default.mask_permissions)?;
            }
            for user in acl_default_user {
                acl.add_entry_full(acl::ACL_USER, Some(user.uid), user.permissions)?;
            }
            for group in acl_default_group {
                acl.add_entry_full(acl::ACL_GROUP, Some(group.gid), group.permissions)?;
            }
            if !acl.is_valid() {
                bail!("Error while restoring ACL - ACL invalid");
            }
            acl.set_file(&proc_path, acl::ACL_TYPE_DEFAULT)?;
        }

        Ok(head)
    }

    fn restore_xattrs_fcaps_fd(&mut self, fd: RawFd, xattrs: Vec<CaFormatXAttr>, fcaps: Option<CaFormatFCaps>) -> Result<(), Error> {
        for xattr in xattrs {
            if let Err(err) = xattr::fsetxattr(fd, xattr) {
                bail!("fsetxattr failed with error: {}", err);
            }
        }
        if let Some(fcaps) = fcaps {
            if let Err(err) = xattr::fsetxattr_fcaps(fd, fcaps) {
                bail!("fsetxattr_fcaps failed with error: {}", err);
            }
        }

        Ok(())
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

    /// Restore an archive into the specified directory.
    ///
    /// The directory is created if it does not exist.
    pub fn restore<F>(
        &mut self,
        path: &Path,
        callback: &F,
    ) -> Result<(), Error>
        where F: Fn(&Path) -> Result<(), Error>
    {

        let _ = std::fs::create_dir(path);

        let dir = match nix::dir::Dir::open(path, nix::fcntl::OFlag::O_DIRECTORY,  nix::sys::stat::Mode::empty()) {
            Ok(dir) => dir,
            Err(err) => bail!("unable to open target directory {:?} - {}", path, err),
        };

        let mut relative_path = PathBuf::new();
        self.restore_sequential(path, &mut relative_path, &OsString::new(), &dir, callback)
    }

    fn restore_sequential<F>(
        &mut self,
        base_path: &Path,
        relative_path: &mut PathBuf,
        filename: &OsStr,  // repeats path last relative_path component
        parent: &nix::dir::Dir,
        callback: &F,
    ) -> Result<(), Error>
        where F: Fn(&Path) -> Result<(), Error>
    {

        let parent_fd = parent.as_raw_fd();

        let full_path = base_path.join(&relative_path);

        (callback)(&full_path)?;

        let head: CaFormatHeader = self.read_item()?;

        if head.htype == PXAR_FORMAT_HARDLINK {
            let (target, _offset) = self.read_hardlink(head.size)?;
            let target_path = base_path.join(&target);
            //println!("HARDLINK: {} {:?} -> {:?}", offset, full_path, target_path);
            hardlink(&target_path, &full_path)?;
            return Ok(());
        }

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
                    Err(err) => bail!("unable to open directory {:?} - {}", full_path, err),
                };
            }

            self.restore_ugid(&entry, dir.as_raw_fd())?;
            // fcaps have to be restored after restore_ugid as chown clears security.capability xattr, see CVE-2015-1350
            let mut head = match self.restore_attributes(&entry, dir.as_raw_fd()) {
                Ok(head) => head,
                Err(err) => bail!("Restoring of directory attributes failed - {}", err),
            };

            while head.htype == CA_FORMAT_FILENAME {
                let name = self.read_filename(head.size)?;
                relative_path.push(&name);
                self.restore_sequential(base_path, relative_path, &name, &dir, callback)?;
                relative_path.pop();

                head = self.read_item()?;
            }

            if head.htype != CA_FORMAT_GOODBYE {
                bail!("got unknown header type inside directory entry {:016x}", head.htype);
            }

            //println!("Skip Goodbye");
            if head.size < HEADER_SIZE { bail!("detected short goodbye table"); }

            self.skip_bytes((head.size - HEADER_SIZE) as usize)?;

            self.restore_mode(&entry, dir.as_raw_fd())?;
            self.restore_mtime(&entry, dir.as_raw_fd())?;

            return Ok(());
        }

        if filename.is_empty() {
            bail!("got empty file name at {:?}", full_path)
        }

        if ifmt == libc::S_IFLNK {
            // fixme: create symlink
            //fixme: restore permission, acls, xattr, ...

            let head: CaFormatHeader = self.read_item()?;
            match head.htype {
                CA_FORMAT_SYMLINK => {
                    let target = self.read_link(head.size)?;
                    //println!("TARGET: {:?}", target);
                    if let Err(err) = symlinkat(&target, parent_fd, filename) {
                        bail!("create symlink {:?} failed - {}", full_path, err);
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
                Err(err) => bail!("open file {:?} failed - {}", full_path, err),
            };

            self.restore_ugid(&entry, file.as_raw_fd())?;
            // fcaps have to be restored after restore_ugid as chown clears security.capability xattr, see CVE-2015-1350
            let head = match self.restore_attributes(&entry, file.as_raw_fd()) {
                Ok(head) => head,
                Err(err) => bail!("Restoring of file attributes failed - {}", err),
            };

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

        if head.htype == PXAR_FORMAT_HARDLINK {
            let (target, offset) = self.read_hardlink(head.size)?;
            if verbose {
                println!("Hardlink: {} {:?}", offset, target);
            }
            return Ok(());
        }

        check_ca_header::<CaFormatEntry>(&head, CA_FORMAT_ENTRY)?;
        let entry: CaFormatEntry = self.read_item()?;

        if verbose {
            println!("Mode: {:08x} {:08x}", entry.mode, (entry.mode as u32) & libc::S_IFDIR);
        }

        let ifmt = (entry.mode as u32) & libc::S_IFMT;

        if ifmt == libc::S_IFDIR {

            let mut entry_count = 0;

            loop {
                let head: CaFormatHeader = self.read_item()?;
                if verbose {
                    print_head(&head);
                }

                // This call covers all the cases of the match statement
                // regarding extended attributes. These calls will never
                // break on the loop and can therefore be handled separately.
                // If the header was matched, true is returned and we can continue
                if self.dump_if_attribute(&head, verbose)? {
                    continue;
                }

                match head.htype {
                    CA_FORMAT_FILENAME =>  {
                        let name = self.read_filename(head.size)?;
                        if verbose { println!("Name: {:?}", name); }
                        entry_count += 1;
                        path.push(&name);
                        self.dump_entry(path, verbose, output)?;
                        path.pop();
                    },
                    CA_FORMAT_GOODBYE => {
                        let table_size = (head.size - HEADER_SIZE) as usize;
                        if verbose {
                            println!("Goodbye: {:?}", path);
                            self.dump_goodby_entries(entry_count, table_size)?;
                        } else {
                            self.skip_bytes(table_size)?;
                        }
                        break;
                    },
                    _ => panic!("got unexpected header type inside directory"),
                }
            }
        } else if (ifmt == libc::S_IFBLK) || (ifmt == libc::S_IFCHR) ||
            (ifmt == libc::S_IFLNK) || (ifmt == libc::S_IFREG)
        {
            loop {
                let head: CaFormatHeader = self.read_item()?;
                if verbose {
                    print_head(&head);
                }

                // This call covers all the cases of the match statement
                // regarding extended attributes. These calls will never
                // break on the loop and can therefore be handled separately.
                // If the header was matched, true is returned and we can continue
                if self.dump_if_attribute(&head, verbose)? {
                    continue;
                }

                match head.htype {
                    CA_FORMAT_SYMLINK => {
                        let target = self.read_link(head.size)?;
                        if verbose {
                            println!("Symlink: {:?}", target);
                        }
                        break;
                    },
                    CA_FORMAT_DEVICE => {
                        let device: CaFormatDevice = self.read_item()?;
                        if verbose {
                            println!("Device: {}, {}", device.major, device.minor);
                        }
                        break;
                    },
                    CA_FORMAT_PAYLOAD => {
                        let payload_size = (head.size - HEADER_SIZE) as usize;
                        if verbose {
                            println!("Payload: {}", payload_size);
                        }
                        self.skip_bytes(payload_size)?;
                        break;
                    }
                    _ => {
                        panic!("got unexpected header type inside non-directory");
                    }
                }
            }
        } else if ifmt == libc::S_IFIFO {
            if verbose {
                println!("Fifo:");
            }
        } else if ifmt == libc::S_IFSOCK {
            if verbose {
                println!("Socket:");
            }
        } else {
            panic!("unknown st_mode");
        }
        Ok(())
    }

    fn dump_if_attribute(&mut self, header: &CaFormatHeader, verbose: bool) -> Result<bool, Error> {
        match header.htype {
            CA_FORMAT_XATTR => {
                let xattr = self.read_xattr((header.size - HEADER_SIZE) as usize)?;
                if verbose && self.has_features(CA_FORMAT_WITH_XATTRS) {
                    println!("XAttr: {:?}", xattr);
                }
            },
            CA_FORMAT_FCAPS => {
                let fcaps = self.read_fcaps((header.size - HEADER_SIZE) as usize)?;
                if verbose && self.has_features(CA_FORMAT_WITH_FCAPS) {
                    println!("FCaps: {:?}", fcaps);
                }
            },
            CA_FORMAT_ACL_USER => {
                let user = self.read_item::<CaFormatACLUser>()?;
                if verbose && self.has_features(CA_FORMAT_WITH_ACL) {
                    println!("ACLUser: {:?}", user);
                }
            },
            CA_FORMAT_ACL_GROUP => {
                let group = self.read_item::<CaFormatACLGroup>()?;
                if verbose && self.has_features(CA_FORMAT_WITH_ACL) {
                    println!("ACLGroup: {:?}", group);
                }
            },
            CA_FORMAT_ACL_GROUP_OBJ => {
                let group_obj = self.read_item::<CaFormatACLGroupObj>()?;
                if verbose && self.has_features(CA_FORMAT_WITH_ACL) {
                    println!("ACLGroupObj: {:?}", group_obj);
                }
            },
            CA_FORMAT_ACL_DEFAULT => {
                let default = self.read_item::<CaFormatACLDefault>()?;
                if verbose && self.has_features(CA_FORMAT_WITH_ACL) {
                    println!("ACLDefault: {:?}", default);
                }
            },
            CA_FORMAT_ACL_DEFAULT_USER => {
                let default_user = self.read_item::<CaFormatACLUser>()?;
                if verbose && self.has_features(CA_FORMAT_WITH_ACL) {
                    println!("ACLDefaultUser: {:?}", default_user);
                }
            },
            CA_FORMAT_ACL_DEFAULT_GROUP => {
                let default_group = self.read_item::<CaFormatACLGroup>()?;
                if verbose && self.has_features(CA_FORMAT_WITH_ACL) {
                    println!("ACLDefaultGroup: {:?}", default_group);
                }
            },
            _ => return Ok(false),
        }

        Ok(true)
    }

    fn dump_goodby_entries(
        &mut self,
        entry_count: usize,
        table_size: usize,
    ) -> Result<(), Error> {

        const GOODBYE_ITEM_SIZE: usize = std::mem::size_of::<CaFormatGoodbyeItem>();

        if table_size < GOODBYE_ITEM_SIZE {
            bail!("Goodbye table to small ({} < {})", table_size, GOODBYE_ITEM_SIZE);
        }
        if (table_size % GOODBYE_ITEM_SIZE) != 0 {
            bail!("Goodbye table with strange size ({})", table_size);
        }

        let entries = table_size / GOODBYE_ITEM_SIZE;

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
            if count >= entries {
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

fn hardlink(oldpath: &Path, newpath: &Path) -> Result<(), Error> {
    oldpath.with_nix_path(|oldpath| {
        newpath.with_nix_path(|newpath| {
            let res = unsafe { libc::link(oldpath.as_ptr(), newpath.as_ptr()) };
            Errno::result(res)?;
            Ok(())
        })?
    })?
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

fn mode_user_to_acl_permissions(mode: u64) -> u64 {
    return (mode >> 6) & 7;
}

fn mode_group_to_acl_permissions(mode: u64) -> u64 {
    return (mode >> 3) & 7;
}

fn mode_other_to_acl_permissions(mode: u64) -> u64 {
    return mode & 7;
}
