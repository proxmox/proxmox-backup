//! *pxar* format encoder.
//!
//! This module contain the code to generate *pxar* archive files.

use failure::*;
use endian_trait::Endian;
use std::collections::HashMap;

use super::format_definition::*;
use super::binary_search_tree::*;
use super::helper::*;
use crate::tools::acl;
use crate::tools::xattr;

use std::io::Write;
use std::os::unix::io::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::io::RawFd;
use std::path::{Path, PathBuf};

use std::ffi::CStr;

use nix::NixPath;
use nix::fcntl::OFlag;
use nix::sys::stat::Mode;
use nix::errno::Errno;
use nix::sys::stat::FileStat;

use crate::tools::vec;

/// The format requires to build sorted directory lookup tables in
/// memory, so we restrict the number of allowed entries to limit
/// maximum memory usage.
pub const MAX_DIRECTORY_ENTRIES: usize = 256*1024;

#[derive(Eq, PartialEq, Hash)]
struct HardLinkInfo {
    st_dev: u64,
    st_ino: u64,
}

pub struct Encoder<'a, W: Write> {
    base_path: PathBuf,
    relative_path: PathBuf,
    writer: &'a mut W,
    writer_pos: usize,
    _size: usize,
    file_copy_buffer: Vec<u8>,
    all_file_systems: bool,
    root_st_dev: u64,
    verbose: bool,
    feature_flags: u64,
    hardlinks: HashMap<HardLinkInfo, (PathBuf, u64)>,
}

impl <'a, W: Write> Encoder<'a, W> {

    // used for error reporting
    fn full_path(&self) ->  PathBuf {
        self.base_path.join(&self.relative_path)
    }

    pub fn encode(
        path: PathBuf,
        dir: &mut nix::dir::Dir,
        writer: &'a mut W,
        all_file_systems: bool,
        verbose: bool,
        feature_flags: u64,
    ) -> Result<(), Error> {

        const FILE_COPY_BUFFER_SIZE: usize = 1024*1024;

        let mut file_copy_buffer = Vec::with_capacity(FILE_COPY_BUFFER_SIZE);
        unsafe { file_copy_buffer.set_len(FILE_COPY_BUFFER_SIZE); }


        // todo: use scandirat??

        let dir_fd = dir.as_raw_fd();
        let stat = match nix::sys::stat::fstat(dir_fd) {
            Ok(stat) => stat,
            Err(err) => bail!("fstat {:?} failed - {}", path, err),
        };

        if !is_directory(&stat) {
            bail!("got unexpected file type {:?} (not a directory)", path);
        }

        let magic = detect_fs_type(dir_fd)?;

        if is_virtual_file_system(magic) {
            bail!("backup virtual file systems is disabled!");
        }

        let mut me = Self {
            base_path: path,
            relative_path: PathBuf::new(),
            writer: writer,
            writer_pos: 0,
            _size: 0,
            file_copy_buffer,
            all_file_systems,
            root_st_dev: stat.st_dev,
            verbose,
            feature_flags,
            hardlinks: HashMap::new(),
        };

        if verbose { println!("{:?}", me.full_path()); }

        me.encode_dir(dir, &stat, magic)?;

        Ok(())
    }

    fn write(&mut self,  buf: &[u8]) -> Result<(), Error> {
        self.writer.write_all(buf)?;
        self.writer_pos += buf.len();
        Ok(())
    }

    fn write_item<T: Endian>(&mut self, item: T) ->  Result<(), Error> {

        let data = item.to_le();

        let buffer = unsafe { std::slice::from_raw_parts(
            &data as *const T as *const u8,
            std::mem::size_of::<T>()
        )};

        self.write(buffer)?;

        Ok(())
    }

    fn flush_copy_buffer(&mut self, size: usize) -> Result<(), Error> {
        self.writer.write_all(&self.file_copy_buffer[..size])?;
        self.writer_pos += size;
        Ok(())
    }

    fn write_header(&mut self, htype: u64, size: u64) -> Result<(), Error> {

        let size = size + (std::mem::size_of::<CaFormatHeader>() as u64);
        self.write_item(CaFormatHeader { size, htype })?;

        Ok(())
    }

    fn write_filename(&mut self, name: &CStr) -> Result<(), Error> {

        let buffer = name.to_bytes_with_nul();
        self.write_header(CA_FORMAT_FILENAME, buffer.len() as u64)?;
        self.write(buffer)?;

        Ok(())
    }

    fn create_entry(&self, stat: &FileStat) -> Result<CaFormatEntry, Error> {

        let mode = if is_symlink(&stat) {
            (libc::S_IFLNK | 0o777) as u64
        } else {
            (stat.st_mode & (libc::S_IFMT | 0o7777)) as u64
        };

        let mtime = stat.st_mtime * 1_000_000_000 + stat.st_mtime_nsec;
        if mtime < 0 {
            bail!("got strange mtime ({}) from fstat for {:?}.", mtime, self.full_path());
        }


        let entry = CaFormatEntry {
            mode: mode,
            flags: 0,
            uid: stat.st_uid as u64,
            gid: stat.st_gid as u64,
            mtime: mtime as u64,
        };

        Ok(entry)
    }

    fn read_chattr(&self, fd: RawFd, entry: &mut CaFormatEntry) -> Result<(), Error> {

        let mut attr: usize = 0;

        let res = unsafe { read_attr_fd(fd, &mut attr)};
        if let Err(err) = res {
            if let nix::Error::Sys(errno) = err {
                if errno_is_unsupported(errno) { return Ok(()) };
            }
            bail!("read_attr_fd failed for {:?} - {}", self.full_path(), err);
        }

        let flags = ca_feature_flags_from_chattr(attr as u32);
        entry.flags = entry.flags | flags;

        Ok(())
    }

    fn read_fat_attr(&self, fd: RawFd, magic: i64, entry: &mut CaFormatEntry) -> Result<(), Error> {

        if magic != MSDOS_SUPER_MAGIC && magic != FUSE_SUPER_MAGIC { return Ok(()); }

        let mut attr: u32 = 0;

        let res = unsafe { read_fat_attr_fd(fd, &mut attr)};
        if let Err(err) = res {
            if let nix::Error::Sys(errno) = err {
                if errno_is_unsupported(errno) { return Ok(()) };
            }
            bail!("read_fat_attr_fd failed for {:?} - {}", self.full_path(), err);
        }

        let flags = ca_feature_flags_from_fat_attr(attr);
        entry.flags = entry.flags | flags;

        Ok(())
    }

    /// True if all of the given feature flags are set in the Encoder, false otherwise
    fn has_features(&self, feature_flags: u64) -> bool {
        (self.feature_flags & feature_flags) == feature_flags
    }

    /// True if at least one of the given feature flags is set in the Encoder, false otherwise
    fn has_some_features(&self, feature_flags: u64) -> bool {
        (self.feature_flags & feature_flags) != 0
    }

    fn read_xattrs(&self, fd: RawFd, stat: &FileStat) -> Result<(Vec<CaFormatXAttr>, Option<CaFormatFCaps>), Error> {
        let mut xattrs = Vec::new();
        let mut fcaps = None;

        let flags = CA_FORMAT_WITH_XATTRS | CA_FORMAT_WITH_FCAPS;
        if !self.has_some_features(flags) {
            return Ok((xattrs, fcaps));
        }
        // Should never be called on symlinks, just in case check anyway
        if is_symlink(&stat) {
            return Ok((xattrs, fcaps));
        }

        let xattr_names = match xattr::flistxattr(fd) {
            Ok(names) => names,
            // Do not bail if the underlying endpoint does not supports xattrs
            Err(Errno::EOPNOTSUPP) => return Ok((xattrs, fcaps)),
            // Do not bail if the endpoint cannot carry xattrs (such as symlinks)
            Err(Errno::EBADF) => return Ok((xattrs, fcaps)),
            Err(err) => bail!("read_xattrs failed for {:?} - {}", self.full_path(), err),
        };

        for name in xattr_names.split(|c| *c == b'\0') {
            // Only extract the relevant extended attributes
            if !xattr::is_valid_xattr_name(&name) {
                continue;
            }

            let value = match xattr::fgetxattr(fd, name) {
                Ok(value) => value,
                // Vanished between flistattr and getxattr, this is ok, silently ignore
                Err(Errno::ENODATA) => continue,
                Err(err) => bail!("read_xattrs failed for {:?} - {}", self.full_path(), err),
            };

            if xattr::is_security_capability(&name) {
                if self.has_features(CA_FORMAT_WITH_FCAPS) {
                    // fcaps are stored in own format within the archive
                    fcaps = Some(CaFormatFCaps {
                        data: value,
                    });
                }
            } else if self.has_features(CA_FORMAT_WITH_XATTRS) {
                xattrs.push(CaFormatXAttr {
                    name: name.to_vec(),
                    value: value,
                });
            }
        }
        xattrs.sort();

        Ok((xattrs, fcaps))
    }

    fn read_acl(&self, fd: RawFd, stat: &FileStat, acl_type: acl::ACLType) -> Result<PxarACL, Error> {
        let ret = PxarACL {
            users: Vec::new(),
            groups: Vec::new(),
            group_obj: None,
            default: None,
        };

        if !self.has_features(CA_FORMAT_WITH_ACL) {
            return Ok(ret);
        }
        if is_symlink(&stat) {
            return Ok(ret);
        }
        if acl_type == acl::ACL_TYPE_DEFAULT && !is_directory(&stat) {
            bail!("ACL_TYPE_DEFAULT only defined for directories.");
        }

        // In order to be able to get ACLs with type ACL_TYPE_DEFAULT, we have
        // to create a path for acl_get_file(). acl_get_fd() only allows to get
        // ACL_TYPE_ACCESS attributes.
        let proc_path = Path::new("/proc/self/fd/").join(fd.to_string());
        let acl = match acl::ACL::get_file(&proc_path, acl_type) {
            Ok(acl) => acl,
            // Don't bail if underlying endpoint does not support acls
            Err(Errno::EOPNOTSUPP) => return Ok(ret),
            // Don't bail if the endpoint cannot carry acls
            Err(Errno::EBADF) => return Ok(ret),
            // Don't bail if there is no data
            Err(Errno::ENODATA) => return Ok(ret),
            Err(err) => bail!("error while reading ACL - {}", err),
        };

        self.process_acl(acl, acl_type)
    }

    fn process_acl(&self, acl: acl::ACL, acl_type: acl::ACLType) -> Result<PxarACL, Error> {
        let mut acl_user = Vec::new();
        let mut acl_group = Vec::new();
        let mut acl_group_obj = None;
        let mut acl_default = None;
        let mut user_obj_permissions = None;
        let mut group_obj_permissions = None;
        let mut other_permissions = None;
        let mut mask_permissions = None;

        for entry in &mut acl.entries() {
            let tag = entry.get_tag_type()?;
            let permissions = entry.get_permissions()?;
            match tag {
                acl::ACL_USER_OBJ => user_obj_permissions = Some(permissions),
                acl::ACL_GROUP_OBJ => group_obj_permissions = Some(permissions),
                acl::ACL_OTHER => other_permissions = Some(permissions),
                acl::ACL_MASK => mask_permissions = Some(permissions),
                acl::ACL_USER => {
                    acl_user.push(CaFormatACLUser {
                        uid: entry.get_qualifier()?,
                        permissions: permissions,
                    });
                },
                acl::ACL_GROUP => {
                    acl_group.push(CaFormatACLGroup {
                        gid: entry.get_qualifier()?,
                        permissions: permissions,
                    });
                },
                _ => bail!("Unexpected ACL tag encountered!"),
            }
        }

        acl_user.sort();
        acl_group.sort();

        match acl_type {
            acl::ACL_TYPE_ACCESS => {
                // The mask permissions are mapped to the stat group permissions
                // in case that the ACL group permissions were set.
                // Only in that case we need to store the group permissions,
                // in the other cases they are identical to the stat group permissions.
                if let (Some(gop), Some(_)) = (group_obj_permissions, mask_permissions) {
                    acl_group_obj = Some(CaFormatACLGroupObj {
                        permissions: gop,
                    });
                }
            },
            acl::ACL_TYPE_DEFAULT => {
                if user_obj_permissions != None ||
                   group_obj_permissions != None ||
                   other_permissions != None ||
                   mask_permissions != None
                {
                    acl_default = Some(CaFormatACLDefault {
                        // The value is set to UINT64_MAX as placeholder if one
                        // of the permissions is not set
                        user_obj_permissions: user_obj_permissions.unwrap_or(std::u64::MAX),
                        group_obj_permissions: group_obj_permissions.unwrap_or(std::u64::MAX),
                        other_permissions: other_permissions.unwrap_or(std::u64::MAX),
                        mask_permissions: mask_permissions.unwrap_or(std::u64::MAX),
                    });
                }
            },
            _ => bail!("Unexpected ACL type encountered"),
        }

        Ok(PxarACL {
            users: acl_user,
            groups: acl_group,
            group_obj: acl_group_obj,
            default: acl_default,
        })
    }

    fn write_entry(&mut self, entry: CaFormatEntry) -> Result<(), Error> {

        self.write_header(CA_FORMAT_ENTRY, std::mem::size_of::<CaFormatEntry>() as u64)?;
        self.write_item(entry)?;

        Ok(())
    }

    fn write_xattr(&mut self, xattr: CaFormatXAttr) -> Result<(), Error> {
        let size = xattr.name.len() + xattr.value.len() + 1; // +1 for '\0' separating name and value
        self.write_header(CA_FORMAT_XATTR, size as u64)?;
        self.write(xattr.name.as_slice())?;
        self.write(&[0])?;
        self.write(xattr.value.as_slice())?;

        Ok(())
    }

    fn write_fcaps(&mut self, fcaps: Option<CaFormatFCaps>) -> Result<(), Error> {
        if let Some(fcaps) = fcaps {
            let size = fcaps.data.len();
            self.write_header(CA_FORMAT_FCAPS, size as u64)?;
            self.write(fcaps.data.as_slice())?;
        }

        Ok(())
    }

    fn write_acl_user(&mut self, acl_user: CaFormatACLUser) -> Result<(), Error> {
        self.write_header(CA_FORMAT_ACL_USER,  std::mem::size_of::<CaFormatACLUser>() as u64)?;
        self.write_item(acl_user)?;

        Ok(())
    }

    fn write_acl_group(&mut self, acl_group: CaFormatACLGroup) -> Result<(), Error> {
        self.write_header(CA_FORMAT_ACL_GROUP,  std::mem::size_of::<CaFormatACLGroup>() as u64)?;
        self.write_item(acl_group)?;

        Ok(())
    }

    fn write_acl_group_obj(&mut self, acl_group_obj: CaFormatACLGroupObj) -> Result<(), Error> {
        self.write_header(CA_FORMAT_ACL_GROUP_OBJ,  std::mem::size_of::<CaFormatACLGroupObj>() as u64)?;
        self.write_item(acl_group_obj)?;

        Ok(())
    }

    fn write_acl_default(&mut self, acl_default: CaFormatACLDefault) -> Result<(), Error> {
        self.write_header(CA_FORMAT_ACL_DEFAULT,  std::mem::size_of::<CaFormatACLDefault>() as u64)?;
        self.write_item(acl_default)?;

        Ok(())
    }

    fn write_acl_default_user(&mut self, acl_default_user: CaFormatACLUser) -> Result<(), Error> {
        self.write_header(CA_FORMAT_ACL_DEFAULT_USER,  std::mem::size_of::<CaFormatACLUser>() as u64)?;
        self.write_item(acl_default_user)?;

        Ok(())
    }

    fn write_acl_default_group(&mut self, acl_default_group: CaFormatACLGroup) -> Result<(), Error> {
        self.write_header(CA_FORMAT_ACL_DEFAULT_GROUP,  std::mem::size_of::<CaFormatACLGroup>() as u64)?;
        self.write_item(acl_default_group)?;

        Ok(())
    }

    fn write_goodbye_table(&mut self, goodbye_offset: usize, goodbye_items: &mut [CaFormatGoodbyeItem]) -> Result<(), Error> {

        goodbye_items.sort_unstable_by(|a, b| a.hash.cmp(&b.hash));

        let item_count = goodbye_items.len();

        let goodbye_table_size = (item_count + 1)*std::mem::size_of::<CaFormatGoodbyeItem>();

        self.write_header(CA_FORMAT_GOODBYE, goodbye_table_size as u64)?;

        if self.file_copy_buffer.len() < goodbye_table_size {
            let need = goodbye_table_size - self.file_copy_buffer.len();
            self.file_copy_buffer.reserve(need);
            unsafe { self.file_copy_buffer.set_len(self.file_copy_buffer.capacity()); }
        }

        let buffer = &mut self.file_copy_buffer;

        copy_binary_search_tree(item_count, |s, d| {
            let item = &goodbye_items[s];
            let offset = d*std::mem::size_of::<CaFormatGoodbyeItem>();
            let dest = crate::tools::map_struct_mut::<CaFormatGoodbyeItem>(&mut buffer[offset..]).unwrap();
            dest.offset = u64::to_le(item.offset);
            dest.size = u64::to_le(item.size);
            dest.hash = u64::to_le(item.hash);
        });

        // append CaFormatGoodbyeTail as last item
        let offset = item_count*std::mem::size_of::<CaFormatGoodbyeItem>();
        let dest = crate::tools::map_struct_mut::<CaFormatGoodbyeItem>(&mut buffer[offset..]).unwrap();
        dest.offset = u64::to_le(goodbye_offset as u64);
        dest.size = u64::to_le((goodbye_table_size + std::mem::size_of::<CaFormatHeader>()) as u64);
        dest.hash = u64::to_le(CA_FORMAT_GOODBYE_TAIL_MARKER);

        self.flush_copy_buffer(goodbye_table_size)?;

        Ok(())
    }

    fn encode_dir(&mut self, dir: &mut nix::dir::Dir, dir_stat: &FileStat, magic: i64)  -> Result<(), Error> {

        //println!("encode_dir: {:?} start {}", self.full_path(), self.writer_pos);

        let mut name_list = vec![];

        let rawfd = dir.as_raw_fd();

        let dir_start_pos = self.writer_pos;

        let mut dir_entry = self.create_entry(&dir_stat)?;

        self.read_chattr(rawfd, &mut dir_entry)?;
        self.read_fat_attr(rawfd, magic, &mut dir_entry)?;
        let (xattrs, fcaps) = self.read_xattrs(rawfd, &dir_stat)?;
        let acl_access = self.read_acl(rawfd, &dir_stat, acl::ACL_TYPE_ACCESS)?;
        let acl_default = self.read_acl(rawfd, &dir_stat, acl::ACL_TYPE_DEFAULT)?;

        self.write_entry(dir_entry)?;
        for xattr in xattrs {
            self.write_xattr(xattr)?;
        }
        self.write_fcaps(fcaps)?;

        for user in acl_access.users {
            self.write_acl_user(user)?;
        }
        for group in acl_access.groups {
            self.write_acl_group(group)?;
        }
        if let Some(group_obj) = acl_access.group_obj {
            self.write_acl_group_obj(group_obj)?;
        }

        for default_user in acl_default.users {
            self.write_acl_default_user(default_user)?;
        }
        for default_group in acl_default.groups {
            self.write_acl_default_group(default_group)?;
        }
        if let Some(default) = acl_default.default {
            self.write_acl_default(default)?;
        }

        let mut dir_count = 0;

        let include_children;
        if is_virtual_file_system(magic) {
            include_children = false;
        } else {
            include_children = (self.root_st_dev == dir_stat.st_dev) || self.all_file_systems;
        }

        if include_children {
            for entry in dir.iter() {
                dir_count += 1;
                if dir_count > MAX_DIRECTORY_ENTRIES {
                    bail!("too many directory items in {:?} (> {})",
                          self.full_path(), MAX_DIRECTORY_ENTRIES);
                }

                let entry = match entry {
                    Ok(entry) => entry,
                    Err(err) => bail!("readir {:?} failed - {}", self.full_path(), err),
                };
                let filename = entry.file_name().to_owned();

                let name = filename.to_bytes_with_nul();
                let name_len = name.len();
                if name_len == 2 && name[0] == b'.' && name[1] == 0u8 { continue; }
                if name_len == 3 && name[0] == b'.' && name[1] == b'.' && name[2] == 0u8 { continue; }

                name_list.push(filename);
            }
        } else {
            eprintln!("skip mount point: {:?}", self.full_path());
        }

        name_list.sort_unstable_by(|a, b| a.cmp(&b));

        let mut goodbye_items = vec![];

        for filename in &name_list {
            self.relative_path.push(std::ffi::OsStr::from_bytes(filename.as_bytes()));

            if self.verbose { println!("{:?}", self.full_path()); }

            let stat = match nix::sys::stat::fstatat(rawfd, filename.as_ref(), nix::fcntl::AtFlags::AT_SYMLINK_NOFOLLOW) {
                Ok(stat) => stat,
                Err(nix::Error::Sys(Errno::ENOENT)) => {
                    self.report_vanished_file(&self.full_path())?;
                    continue;
                }
                Err(err) => bail!("fstat {:?} failed - {}", self.full_path(), err),
            };

            let start_pos = self.writer_pos;

            if is_directory(&stat) {

                let mut dir = match nix::dir::Dir::openat(rawfd, filename.as_ref(), OFlag::O_DIRECTORY|OFlag::O_NOFOLLOW, Mode::empty()) {
                    Ok(dir) => dir,
                    Err(nix::Error::Sys(Errno::ENOENT)) => {
                        self.report_vanished_file(&self.full_path())?;
                        continue; // fixme!!
                    },
                    Err(err) => bail!("open dir {:?} failed - {}", self.full_path(), err),
                };

                let child_magic = if dir_stat.st_dev != stat.st_dev {
                    detect_fs_type(dir.as_raw_fd())?
                } else {
                    magic
                };

                self.write_filename(&filename)?;
                self.encode_dir(&mut dir, &stat, child_magic)?;

            } else if is_reg_file(&stat) {

                let mut hardlink_target = None;

                if stat.st_nlink > 1 {
                    let link_info = HardLinkInfo { st_dev: stat.st_dev, st_ino: stat.st_ino };
                    hardlink_target = self.hardlinks.get(&link_info).map(|(v, offset)| {
                        let mut target = v.clone().into_os_string();
                        target.push("\0"); // add Nul byte
                        (target, (start_pos as u64) - offset)
                    });
                    if hardlink_target == None {
                        self.hardlinks.insert(link_info, (self.relative_path.clone(), start_pos as u64));
                    }
                }

                if let Some((target, offset)) = hardlink_target {

                    self.write_filename(&filename)?;
                    self.encode_hardlink(target.as_bytes(), offset)?;

                } else {

                    let filefd = match nix::fcntl::openat(rawfd, filename.as_ref(), OFlag::O_NOFOLLOW, Mode::empty()) {
                        Ok(filefd) => filefd,
                        Err(nix::Error::Sys(Errno::ENOENT)) => {
                            self.report_vanished_file(&self.full_path())?;
                            continue;
                        },
                        Err(err) => bail!("open file {:?} failed - {}", self.full_path(), err),
                    };

                    let child_magic = if dir_stat.st_dev != stat.st_dev {
                        detect_fs_type(filefd)?
                    } else {
                        magic
                    };

                    self.write_filename(&filename)?;
                    let res = self.encode_file(filefd, &stat, child_magic);
                    let _ = nix::unistd::close(filefd); // ignore close errors
                    res?;
                }

            } else if is_symlink(&stat) {
                let mut buffer = vec::undefined(libc::PATH_MAX as usize);

                let res = filename.with_nix_path(|cstr| {
                    unsafe { libc::readlinkat(rawfd, cstr.as_ptr(), buffer.as_mut_ptr() as *mut libc::c_char, buffer.len()-1) }
                })?;

                match Errno::result(res) {
                    Ok(len) => {
                        buffer[len as usize] = 0u8; // add Nul byte
                        self.write_filename(&filename)?;
                        self.encode_symlink(&buffer[..((len+1) as usize)], &stat)?
                    }
                    Err(nix::Error::Sys(Errno::ENOENT)) => {
                        self.report_vanished_file(&self.full_path())?;
                        continue;
                    }
                    Err(err) => bail!("readlink {:?} failed - {}", self.full_path(), err),
                }
            } else if is_block_dev(&stat) || is_char_dev(&stat) {
                self.write_filename(&filename)?;
                self.encode_device(&stat)?;
            } else if is_fifo(&stat) || is_socket(&stat) {
                self.write_filename(&filename)?;
                self.encode_special(&stat)?;
            } else {
                bail!("unsupported file type (mode {:o} {:?})", stat.st_mode, self.full_path());
            }

            let end_pos = self.writer_pos;

            goodbye_items.push(CaFormatGoodbyeItem {
                offset: start_pos as u64,
                size: (end_pos - start_pos) as u64,
                hash: compute_goodbye_hash(filename.to_bytes()),
            });

            self.relative_path.pop();
        }

        //println!("encode_dir: {:?} end {}", self.full_path(), self.writer_pos);

        // fixup goodby item offsets
        let goodbye_start = self.writer_pos as u64;
        for item in &mut goodbye_items {
            item.offset = goodbye_start - item.offset;
        }

        let goodbye_offset = self.writer_pos - dir_start_pos;

        self.write_goodbye_table(goodbye_offset, &mut goodbye_items)?;

        //println!("encode_dir: {:?} end1 {}", self.full_path(), self.writer_pos);
        Ok(())
    }

    fn encode_file(&mut self, filefd: RawFd, stat: &FileStat, magic: i64)  -> Result<(), Error> {

        //println!("encode_file: {:?}", self.full_path());

        let mut entry = self.create_entry(&stat)?;

        self.read_chattr(filefd, &mut entry)?;
        self.read_fat_attr(filefd, magic, &mut entry)?;
        let (xattrs, fcaps) = self.read_xattrs(filefd, &stat)?;
        let acl_access = self.read_acl(filefd, &stat, acl::ACL_TYPE_ACCESS)?;

        self.write_entry(entry)?;
        for xattr in xattrs {
            self.write_xattr(xattr)?;
        }
        self.write_fcaps(fcaps)?;
        for user in acl_access.users {
            self.write_acl_user(user)?;
        }
        for group in acl_access.groups {
            self.write_acl_group(group)?;
        }
        if let Some(group_obj) = acl_access.group_obj {
            self.write_acl_group_obj(group_obj)?;
        }

        let include_payload;
        if is_virtual_file_system(magic) {
            include_payload = false;
        } else {
            include_payload = (stat.st_dev == self.root_st_dev) || self.all_file_systems;
        }

        if !include_payload {
            eprintln!("skip content: {:?}", self.full_path());
            self.write_header(CA_FORMAT_PAYLOAD, 0)?;
            return Ok(());
        }

        let size = stat.st_size as u64;

        self.write_header(CA_FORMAT_PAYLOAD, size)?;

        let mut pos: u64 = 0;
        loop {
            let n = match nix::unistd::read(filefd, &mut self.file_copy_buffer) {
                Ok(n) => n,
                Err(nix::Error::Sys(Errno::EINTR)) => continue /* try again */,
                Err(err) =>  bail!("read {:?} failed - {}", self.full_path(), err),
            };
            if n == 0 { // EOF
                if pos != size {
                    // Note:: casync format cannot handle that
                    bail!("detected shrinked file {:?} ({} < {})", self.full_path(), pos, size);
                }
                break;
            }

            let mut next = pos + (n as u64);

            if next > size { next = size; }

            let count = (next - pos) as usize;

            self.flush_copy_buffer(count)?;

            pos = next;

            if pos >= size { break; }
        }

        Ok(())
    }

    fn encode_device(&mut self, stat: &FileStat)  -> Result<(), Error> {

        let entry = self.create_entry(&stat)?;

        self.write_entry(entry)?;

        let major = unsafe { libc::major(stat.st_rdev) } as u64;
        let minor = unsafe { libc::minor(stat.st_rdev) } as u64;

        //println!("encode_device: {:?} {} {} {}", self.full_path(), stat.st_rdev, major, minor);

        self.write_header(CA_FORMAT_DEVICE, std::mem::size_of::<CaFormatDevice>() as u64)?;
        self.write_item(CaFormatDevice { major, minor })?;

        Ok(())
    }

    // FIFO or Socket
    fn encode_special(&mut self, stat: &FileStat)  -> Result<(), Error> {

        let entry = self.create_entry(&stat)?;

        self.write_entry(entry)?;

        Ok(())
    }

    fn encode_symlink(&mut self, target: &[u8], stat: &FileStat)  -> Result<(), Error> {

        //println!("encode_symlink: {:?} -> {:?}", self.full_path(), target);

        let entry = self.create_entry(&stat)?;
        self.write_entry(entry)?;

        self.write_header(CA_FORMAT_SYMLINK, target.len() as u64)?;
        self.write(target)?;

        Ok(())
    }

    fn encode_hardlink(&mut self, target: &[u8], offset: u64)  -> Result<(), Error> {

        //println!("encode_hardlink: {:?} -> {:?}", self.full_path(), target);

        // Note: HARDLINK replaces an ENTRY.
        self.write_header(PXAR_FORMAT_HARDLINK, (target.len() as u64) + 8)?;
        self.write_item(offset)?;
        self.write(target)?;

        Ok(())
    }

    // the report_XXX method may raise and error - depending on encoder configuration

    fn report_vanished_file(&self, path: &Path) -> Result<(), Error> {

        eprintln!("WARNING: detected vanished file {:?}", path);

        Ok(())
    }
}

fn errno_is_unsupported(errno: Errno) -> bool {

    match errno {
        Errno::ENOTTY | Errno::ENOSYS | Errno::EBADF | Errno::EOPNOTSUPP | Errno::EINVAL => {
            true
        }
        _ => false,
    }
}

fn detect_fs_type(fd: RawFd) -> Result<i64, Error> {
    let mut fs_stat: libc::statfs = unsafe { std::mem::uninitialized() };
    let res = unsafe { libc::fstatfs(fd, &mut fs_stat) };
    Errno::result(res)?;

    Ok(fs_stat.f_type)
}

use nix::{convert_ioctl_res, request_code_read, ioc};

// /usr/include/linux/fs.h: #define FS_IOC_GETFLAGS _IOR('f', 1, long)
/// read Linux file system attributes (see man chattr)
nix::ioctl_read!(read_attr_fd, b'f', 1, usize);

// /usr/include/linux/msdos_fs.h: #define FAT_IOCTL_GET_ATTRIBUTES _IOR('r', 0x10, __u32)
// read FAT file system attributes
nix::ioctl_read!(read_fat_attr_fd, b'r', 0x10, u32);


// from /usr/include/linux/magic.h
// and from casync util.h
pub const BINFMTFS_MAGIC: i64 =        0x42494e4d;
pub const CGROUP2_SUPER_MAGIC: i64 =   0x63677270;
pub const CGROUP_SUPER_MAGIC: i64 =    0x0027e0eb;
pub const CONFIGFS_MAGIC: i64 =        0x62656570;
pub const DEBUGFS_MAGIC: i64 =         0x64626720;
pub const DEVPTS_SUPER_MAGIC: i64 =    0x00001cd1;
pub const EFIVARFS_MAGIC: i64 =        0xde5e81e4;
pub const FUSE_CTL_SUPER_MAGIC: i64 =  0x65735543;
pub const HUGETLBFS_MAGIC: i64 =       0x958458f6;
pub const MQUEUE_MAGIC: i64 =          0x19800202;
pub const NFSD_MAGIC: i64 =            0x6e667364;
pub const PROC_SUPER_MAGIC: i64 =      0x00009fa0;
pub const PSTOREFS_MAGIC: i64 =        0x6165676C;
pub const RPCAUTH_GSSMAGIC: i64 =      0x67596969;
pub const SECURITYFS_MAGIC: i64 =      0x73636673;
pub const SELINUX_MAGIC: i64 =         0xf97cff8c;
pub const SMACK_MAGIC: i64 =           0x43415d53;
pub const RAMFS_MAGIC: i64 =           0x858458f6;
pub const TMPFS_MAGIC: i64 =           0x01021994;
pub const SYSFS_MAGIC: i64 =           0x62656572;
pub const MSDOS_SUPER_MAGIC: i64 =     0x00004d44;
pub const FUSE_SUPER_MAGIC: i64 =      0x65735546;


#[inline(always)]
pub fn is_temporary_file_system(magic: i64) -> bool {
    magic == RAMFS_MAGIC || magic == TMPFS_MAGIC
}

pub fn is_virtual_file_system(magic: i64) -> bool {

    match magic {
        BINFMTFS_MAGIC |
        CGROUP2_SUPER_MAGIC |
        CGROUP_SUPER_MAGIC |
        CONFIGFS_MAGIC |
        DEBUGFS_MAGIC |
        DEVPTS_SUPER_MAGIC |
        EFIVARFS_MAGIC |
        FUSE_CTL_SUPER_MAGIC |
        HUGETLBFS_MAGIC |
        MQUEUE_MAGIC |
        NFSD_MAGIC |
        PROC_SUPER_MAGIC |
        PSTOREFS_MAGIC |
        RPCAUTH_GSSMAGIC |
        SECURITYFS_MAGIC |
        SELINUX_MAGIC |
        SMACK_MAGIC |
        SYSFS_MAGIC => true,
        _ => false
    }
}
