use std::collections::{HashSet, HashMap};
use std::convert::TryFrom;
use std::ffi::{CStr, CString, OsStr};
use std::fmt;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd, RawFd};
use std::path::{Path, PathBuf};

use anyhow::{bail, format_err, Error};
use nix::dir::Dir;
use nix::errno::Errno;
use nix::fcntl::OFlag;
use nix::sys::stat::{FileStat, Mode};

use pathpatterns::{MatchEntry, MatchList, MatchType, PatternFlag};
use pxar::Metadata;
use pxar::encoder::LinkOffset;

use proxmox::sys::error::SysError;
use proxmox::tools::fd::RawFdNum;

use crate::pxar::catalog::BackupCatalogWriter;
use crate::pxar::flags;
use crate::pxar::tools::assert_single_path_component;
use crate::tools::{acl, fs, xattr, Fd};

fn detect_fs_type(fd: RawFd) -> Result<i64, Error> {
    let mut fs_stat = std::mem::MaybeUninit::uninit();
    let res = unsafe { libc::fstatfs(fd, fs_stat.as_mut_ptr()) };
    Errno::result(res)?;
    let fs_stat = unsafe { fs_stat.assume_init() };

    Ok(fs_stat.f_type)
}

pub fn is_virtual_file_system(magic: i64) -> bool {
    use proxmox::sys::linux::magic::*;

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

#[derive(Debug)]
struct ArchiveError {
    path: PathBuf,
    error: Error,
}

impl ArchiveError {
    fn new(path: PathBuf, error: Error) -> Self {
        Self { path, error }
    }
}

impl std::error::Error for ArchiveError {}

impl fmt::Display for ArchiveError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "error at {:?}: {}", self.path, self.error)
    }
}

#[derive(Eq, PartialEq, Hash)]
struct HardLinkInfo {
    st_dev: u64,
    st_ino: u64,
}

struct Archiver<'a, 'b> {
    /// FIXME: use bitflags!() for feature_flags
    feature_flags: u64,
    fs_feature_flags: u64,
    fs_magic: i64,
    patterns: &'a [MatchEntry],
    callback: &'a mut dyn FnMut(&Path) -> Result<(), Error>,
    catalog: Option<&'b mut dyn BackupCatalogWriter>,
    path: PathBuf,
    entry_counter: usize,
    entry_limit: usize,
    current_st_dev: libc::dev_t,
    device_set: Option<HashSet<u64>>,
    hardlinks: HashMap<HardLinkInfo, (PathBuf, LinkOffset)>,
}

type Encoder<'a, 'b> = pxar::encoder::Encoder<'a, &'b mut dyn pxar::encoder::SeqWrite>;

pub fn create_archive<T, F>(
    source_dir: Dir,
    mut writer: T,
    mut patterns: Vec<MatchEntry>,
    feature_flags: u64,
    mut device_set: Option<HashSet<u64>>,
    skip_lost_and_found: bool,
    mut callback: F,
    entry_limit: usize,
    catalog: Option<&mut dyn BackupCatalogWriter>,
) -> Result<(), Error>
where
    T: pxar::encoder::SeqWrite,
    F: FnMut(&Path) -> Result<(), Error>,
{
    let fs_magic = detect_fs_type(source_dir.as_raw_fd())?;
    if is_virtual_file_system(fs_magic) {
        bail!("refusing to backup a virtual file system");
    }

    let fs_feature_flags = flags::feature_flags_from_magic(fs_magic);

    let stat = nix::sys::stat::fstat(source_dir.as_raw_fd())?;
    let metadata = get_metadata(
        source_dir.as_raw_fd(),
        &stat,
        feature_flags & fs_feature_flags,
        fs_magic,
    )
    .map_err(|err| format_err!("failed to get metadata for source directory: {}", err))?;

    if let Some(ref mut set) = device_set {
        set.insert(stat.st_dev);
    }

    let writer = &mut writer as &mut dyn pxar::encoder::SeqWrite;
    let mut encoder = Encoder::new(writer, &metadata)?;

    if skip_lost_and_found {
        patterns.push(MatchEntry::parse_pattern(
            "**/lost+found",
            PatternFlag::PATH_NAME,
            MatchType::Exclude,
        )?);
    }

    let mut archiver = Archiver {
        feature_flags,
        fs_feature_flags,
        fs_magic,
        callback: &mut callback,
        patterns: &patterns,
        catalog,
        path: PathBuf::new(),
        entry_counter: 0,
        entry_limit,
        current_st_dev: stat.st_dev,
        device_set,
        hardlinks: HashMap::new(),
    };

    if !patterns.is_empty() {
        let content = generate_pxar_excludes_cli(&patterns);
        let mut file = encoder.create_file(
            &Metadata::default(),
            ".pxarexclude-cli",
            content.len() as u64,
        )?;

        use std::io::Write;
        file.write_all(&content)?;
    }

    archiver.archive_dir_contents(&mut encoder, source_dir)?;
    encoder.finish()?;
    Ok(())
}

struct FileListEntry {
    name: CString,
    path: PathBuf,
    stat: FileStat,
}

impl<'a, 'b> Archiver<'a, 'b> {
    fn flags(&self) -> u64 {
        self.feature_flags & self.fs_feature_flags
    }

    fn wrap_err(&self, err: Error) -> Error {
        if err.downcast_ref::<ArchiveError>().is_some() {
            err
        } else {
            ArchiveError::new(self.path.clone(), err).into()
        }
    }

    fn archive_dir_contents(&mut self, encoder: &mut Encoder, mut dir: Dir) -> Result<(), Error> {
        let entry_counter = self.entry_counter;

        let file_list = self.generate_directory_file_list(&mut dir)?;

        let dir_fd = dir.as_raw_fd();

        let old_path = std::mem::take(&mut self.path);
        for file_entry in file_list {
            (self.callback)(Path::new(OsStr::from_bytes(file_entry.name.to_bytes())))?;
            self.path = file_entry.path;
            self.add_entry(encoder, dir_fd, &file_entry.name, &file_entry.stat)
                .map_err(|err| self.wrap_err(err))?;
        }
        self.path = old_path;
        self.entry_counter = entry_counter;

        Ok(())
    }

    fn generate_directory_file_list(&mut self, dir: &mut Dir) -> Result<Vec<FileListEntry>, Error> {
        let dir_fd = dir.as_raw_fd();

        let mut file_list = Vec::new();

        for file in dir.iter() {
            let file = file?;

            let file_name = file.file_name().to_owned();
            let file_name_bytes = file_name.to_bytes();
            if file_name_bytes == b"." || file_name_bytes == b".." {
                continue;
            }

            if file_name_bytes == b".pxarexclude" {
                // FIXME: handle this file!
                continue;
            }

            let os_file_name = OsStr::from_bytes(file_name_bytes);
            assert_single_path_component(os_file_name)?;
            let full_path = self.path.join(os_file_name);

            let stat = match nix::sys::stat::fstatat(
                dir_fd,
                file_name.as_c_str(),
                nix::fcntl::AtFlags::AT_SYMLINK_NOFOLLOW,
            ) {
                Ok(stat) => stat,
                Err(ref err) if err.not_found() => continue,
                Err(err) => bail!("stat failed on {:?}: {}", full_path, err),
            };

            if self
                .patterns
                .matches(full_path.as_os_str().as_bytes(), Some(stat.st_mode as u32))
                == Some(MatchType::Exclude)
            {
                continue;
            }

            self.entry_counter += 1;
            if self.entry_counter > self.entry_limit {
                bail!("exceeded allowed number of file entries (> {})",self.entry_limit);
            }

            file_list.push(FileListEntry {
                name: file_name,
                path: full_path,
                stat
            });
        }

        file_list.sort_unstable_by(|a, b| a.name.cmp(&b.name));

        Ok(file_list)
    }

    fn add_entry(
        &mut self,
        encoder: &mut Encoder,
        parent: RawFd,
        c_file_name: &CStr,
        stat: &FileStat,
    ) -> Result<(), Error> {
        use pxar::format::mode;

        let file_mode = stat.st_mode & libc::S_IFMT;
        let open_mode = if !(file_mode == libc::S_IFREG || file_mode == libc::S_IFDIR) {
            OFlag::O_PATH
        } else {
            OFlag::empty()
        };

        let fd = Fd::openat(
            &unsafe { RawFdNum::from_raw_fd(parent) },
            c_file_name,
            open_mode | OFlag::O_RDONLY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC | OFlag::O_NOCTTY,
            Mode::empty(),
        )?;

        let metadata = get_metadata(fd.as_raw_fd(), &stat, self.flags(), self.fs_magic)?;

        if self
            .patterns
            .matches(self.path.as_os_str().as_bytes(), Some(stat.st_mode as u32))
            == Some(MatchType::Exclude)
        {
            return Ok(());
        }

        let file_name: &Path = OsStr::from_bytes(c_file_name.to_bytes()).as_ref();
        match metadata.file_type() {
            mode::IFREG => {
                let link_info = HardLinkInfo {
                    st_dev: stat.st_dev,
                    st_ino: stat.st_ino,
                };

                if stat.st_nlink > 1 {
                    if let Some((path, offset)) = self.hardlinks.get(&link_info) {
                        if let Some(ref mut catalog) = self.catalog {
                            catalog.add_hardlink(c_file_name)?;
                        }

                        encoder.add_hardlink(file_name, path, *offset)?;

                        return Ok(());
                    }
                }

                let file_size = stat.st_size as u64;
                if let Some(ref mut catalog) = self.catalog {
                    catalog.add_file(c_file_name, file_size, metadata.stat.mtime)?;
                }

                let offset: LinkOffset =
                    self.add_regular_file(encoder, fd, file_name, &metadata, file_size)?;

                if stat.st_nlink > 1 {
                    self.hardlinks.insert(link_info, (self.path.clone(), offset));
                }

                Ok(())
            }
            mode::IFDIR => {
                let dir = Dir::from_fd(fd.into_raw_fd())?;
                self.add_directory(encoder, dir, c_file_name, &metadata, stat)
            }
            mode::IFSOCK => {
                if let Some(ref mut catalog) = self.catalog {
                    catalog.add_socket(c_file_name)?;
                }

                Ok(encoder.add_socket(&metadata, file_name)?)
            }
            mode::IFIFO => {
                if let Some(ref mut catalog) = self.catalog {
                    catalog.add_fifo(c_file_name)?;
                }

                Ok(encoder.add_fifo(&metadata, file_name)?)
            }
            mode::IFLNK => {
                if let Some(ref mut catalog) = self.catalog {
                    catalog.add_symlink(c_file_name)?;
                }

                self.add_symlink(encoder, fd, file_name, &metadata)
            }
            mode::IFBLK => {
                if let Some(ref mut catalog) = self.catalog {
                    catalog.add_block_device(c_file_name)?;
                }

                self.add_device(encoder, file_name, &metadata, &stat)
            }
            mode::IFCHR => {
                if let Some(ref mut catalog) = self.catalog {
                    catalog.add_char_device(c_file_name)?;
                }

                self.add_device(encoder, file_name, &metadata, &stat)
            }
            other => bail!(
                "encountered unknown file type: 0x{:x} (0o{:o})",
                other,
                other
            ),
        }
    }

    fn add_directory(
        &mut self,
        encoder: &mut Encoder,
        dir: Dir,
        dir_name: &CStr,
        metadata: &Metadata,
        stat: &FileStat,
    ) -> Result<(), Error> {
        let dir_name = OsStr::from_bytes(dir_name.to_bytes());

        let mut encoder = encoder.create_directory(dir_name, &metadata)?;

        let old_fs_magic = self.fs_magic;
        let old_fs_feature_flags = self.fs_feature_flags;
        let old_st_dev = self.current_st_dev;

        let mut skip_contents = false;
        if old_st_dev != stat.st_dev {
            self.fs_magic = detect_fs_type(dir.as_raw_fd())?;
            self.fs_feature_flags = flags::feature_flags_from_magic(self.fs_magic);
            self.current_st_dev = stat.st_dev;

            if is_virtual_file_system(self.fs_magic) {
                skip_contents = true;
            } else if let Some(set) = &self.device_set {
                skip_contents = !set.contains(&stat.st_dev);
            }
        }

        let result = if skip_contents {
            Ok(())
        } else {
            self.archive_dir_contents(&mut encoder, dir)
        };

        self.fs_magic = old_fs_magic;
        self.fs_feature_flags = old_fs_feature_flags;
        self.current_st_dev = old_st_dev;

        encoder.finish()?;
        result
    }

    fn add_regular_file(
        &mut self,
        encoder: &mut Encoder,
        fd: Fd,
        file_name: &Path,
        metadata: &Metadata,
        file_size: u64,
    ) -> Result<LinkOffset, Error> {
        let mut file = unsafe { std::fs::File::from_raw_fd(fd.into_raw_fd()) };
        let offset = encoder.add_file(metadata, file_name, file_size, &mut file)?;
        Ok(offset)
    }

    fn add_symlink(
        &mut self,
        encoder: &mut Encoder,
        fd: Fd,
        file_name: &Path,
        metadata: &Metadata,
    ) -> Result<(), Error> {
        let dest = nix::fcntl::readlinkat(fd.as_raw_fd(), &b""[..])?;
        encoder.add_symlink(metadata, file_name, dest)?;
        Ok(())
    }

    fn add_device(
        &mut self,
        encoder: &mut Encoder,
        file_name: &Path,
        metadata: &Metadata,
        stat: &FileStat,
    ) -> Result<(), Error> {
        Ok(encoder.add_device(
            metadata,
            file_name,
            pxar::format::Device::from_dev_t(stat.st_rdev),
        )?)
    }
}

fn get_metadata(fd: RawFd, stat: &FileStat, flags: u64, fs_magic: i64) -> Result<Metadata, Error> {
    // required for some of these
    let proc_path = Path::new("/proc/self/fd/").join(fd.to_string());

    let mtime = u64::try_from(stat.st_mtime * 1_000_000_000 + stat.st_mtime_nsec)
        .map_err(|_| format_err!("file with negative mtime"))?;

    let mut meta = Metadata {
        stat: pxar::Stat {
            mode: u64::from(stat.st_mode),
            flags: 0,
            uid: stat.st_uid,
            gid: stat.st_gid,
            mtime,
        },
        ..Default::default()
    };

    get_xattr_fcaps_acl(&mut meta, fd, &proc_path, flags)?;
    get_chattr(&mut meta, fd)?;
    get_fat_attr(&mut meta, fd, fs_magic)?;
    get_quota_project_id(&mut meta, fd, flags, fs_magic)?;
    Ok(meta)
}

fn errno_is_unsupported(errno: Errno) -> bool {
    match errno {
        Errno::ENOTTY | Errno::ENOSYS | Errno::EBADF | Errno::EOPNOTSUPP | Errno::EINVAL => true,
        _ => false,
    }
}

fn get_fcaps(meta: &mut Metadata, fd: RawFd, flags: u64) -> Result<(), Error> {
    if 0 == (flags & flags::WITH_FCAPS) {
        return Ok(());
    }

    match xattr::fgetxattr(fd, xattr::xattr_name_fcaps()) {
        Ok(data) => {
            meta.fcaps = Some(pxar::format::FCaps { data });
            Ok(())
        }
        Err(Errno::ENODATA) => Ok(()),
        Err(Errno::EOPNOTSUPP) => Ok(()),
        Err(Errno::EBADF) => Ok(()), // symlinks
        Err(err) => bail!("failed to read file capabilities: {}", err),
    }
}

fn get_xattr_fcaps_acl(
    meta: &mut Metadata,
    fd: RawFd,
    proc_path: &Path,
    flags: u64,
) -> Result<(), Error> {
    if 0 == (flags & flags::WITH_XATTRS) {
        return Ok(());
    }

    let xattrs = match xattr::flistxattr(fd) {
        Ok(names) => names,
        Err(Errno::EOPNOTSUPP) => return Ok(()),
        Err(Errno::EBADF) => return Ok(()), // symlinks
        Err(err) => bail!("failed to read xattrs: {}", err),
    };

    for attr in &xattrs {
        if xattr::is_security_capability(&attr) {
            get_fcaps(meta, fd, flags)?;
            continue;
        }

        if xattr::is_acl(&attr) {
            get_acl(meta, proc_path, flags)?;
            continue;
        }

        if !xattr::is_valid_xattr_name(&attr) {
            continue;
        }

        match xattr::fgetxattr(fd, attr) {
            Ok(data) => meta
                .xattrs
                .push(pxar::format::XAttr::new(attr.to_bytes(), data)),
            Err(Errno::ENODATA) => (), // it got removed while we were iterating...
            Err(Errno::EOPNOTSUPP) => (), // shouldn't be possible so just ignore this
            Err(Errno::EBADF) => (),   // symlinks, shouldn't be able to reach this either
            Err(err) => bail!("error reading extended attribute {:?}: {}", attr, err),
        }
    }

    Ok(())
}

fn get_chattr(metadata: &mut Metadata, fd: RawFd) -> Result<(), Error> {
    let mut attr: usize = 0;

    match unsafe { fs::read_attr_fd(fd, &mut attr) } {
        Ok(_) => (),
        Err(nix::Error::Sys(errno)) if errno_is_unsupported(errno) => {
            return Ok(());
        }
        Err(err) => bail!("failed to read file attributes: {}", err),
    }

    metadata.stat.flags |= flags::feature_flags_from_chattr(attr as u32);

    Ok(())
}

fn get_fat_attr(metadata: &mut Metadata, fd: RawFd, fs_magic: i64) -> Result<(), Error> {
    use proxmox::sys::linux::magic::*;

    if fs_magic != MSDOS_SUPER_MAGIC && fs_magic != FUSE_SUPER_MAGIC {
        return Ok(());
    }

    let mut attr: u32 = 0;

    match unsafe { fs::read_fat_attr_fd(fd, &mut attr) } {
        Ok(_) => (),
        Err(nix::Error::Sys(errno)) if errno_is_unsupported(errno) => {
            return Ok(());
        }
        Err(err) => bail!("failed to read fat attributes: {}", err),
    }

    metadata.stat.flags |= flags::feature_flags_from_fat_attr(attr);

    Ok(())
}

/// Read the quota project id for an inode, supported on ext4/XFS/FUSE/ZFS filesystems
fn get_quota_project_id(
    metadata: &mut Metadata,
    fd: RawFd,
    flags: u64,
    magic: i64,
) -> Result<(), Error> {
    if !(metadata.is_dir() || metadata.is_regular_file()) {
        return Ok(());
    }

    if 0 == (flags & flags::WITH_QUOTA_PROJID) {
        return Ok(());
    }

    use proxmox::sys::linux::magic::*;

    match magic {
        EXT4_SUPER_MAGIC | XFS_SUPER_MAGIC | FUSE_SUPER_MAGIC | ZFS_SUPER_MAGIC => (),
        _ => return Ok(()),
    }

    let mut fsxattr = fs::FSXAttr::default();
    let res = unsafe { fs::fs_ioc_fsgetxattr(fd, &mut fsxattr) };

    // On some FUSE filesystems it can happen that ioctl is not supported.
    // For these cases projid is set to 0 while the error is ignored.
    if let Err(err) = res {
        let errno = err
            .as_errno()
            .ok_or_else(|| format_err!("error while reading quota project id"))?;
        if errno_is_unsupported(errno) {
            return Ok(());
        } else {
            bail!("error while reading quota project id ({})", errno);
        }
    }

    let projid = fsxattr.fsx_projid as u64;
    if projid != 0 {
        metadata.quota_project_id = Some(pxar::format::QuotaProjectId { projid });
    }
    Ok(())
}

fn get_acl(metadata: &mut Metadata, proc_path: &Path, flags: u64) -> Result<(), Error> {
    if 0 == (flags & flags::WITH_ACL) {
        return Ok(());
    }

    if metadata.is_symlink() {
        return Ok(());
    }

    get_acl_do(metadata, proc_path, acl::ACL_TYPE_ACCESS)?;

    if metadata.is_dir() {
        get_acl_do(metadata, proc_path, acl::ACL_TYPE_DEFAULT)?;
    }

    Ok(())
}

fn get_acl_do(
    metadata: &mut Metadata,
    proc_path: &Path,
    acl_type: acl::ACLType,
) -> Result<(), Error> {
    // In order to be able to get ACLs with type ACL_TYPE_DEFAULT, we have
    // to create a path for acl_get_file(). acl_get_fd() only allows to get
    // ACL_TYPE_ACCESS attributes.
    let acl = match acl::ACL::get_file(&proc_path, acl_type) {
        Ok(acl) => acl,
        // Don't bail if underlying endpoint does not support acls
        Err(Errno::EOPNOTSUPP) => return Ok(()),
        // Don't bail if the endpoint cannot carry acls
        Err(Errno::EBADF) => return Ok(()),
        // Don't bail if there is no data
        Err(Errno::ENODATA) => return Ok(()),
        Err(err) => bail!("error while reading ACL - {}", err),
    };

    process_acl(metadata, acl, acl_type)
}

fn process_acl(
    metadata: &mut Metadata,
    acl: acl::ACL,
    acl_type: acl::ACLType,
) -> Result<(), Error> {
    use pxar::format::acl as pxar_acl;
    use pxar::format::acl::{Group, GroupObject, Permissions, User};

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
            acl::ACL_USER_OBJ => user_obj_permissions = Some(Permissions(permissions)),
            acl::ACL_GROUP_OBJ => group_obj_permissions = Some(Permissions(permissions)),
            acl::ACL_OTHER => other_permissions = Some(Permissions(permissions)),
            acl::ACL_MASK => mask_permissions = Some(Permissions(permissions)),
            acl::ACL_USER => {
                acl_user.push(User {
                    uid: entry.get_qualifier()?,
                    permissions: Permissions(permissions),
                });
            }
            acl::ACL_GROUP => {
                acl_group.push(Group {
                    gid: entry.get_qualifier()?,
                    permissions: Permissions(permissions),
                });
            }
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
            if let (Some(gop), true) = (group_obj_permissions, mask_permissions.is_some()) {
                acl_group_obj = Some(GroupObject { permissions: gop });
            }

            metadata.acl.users = acl_user;
            metadata.acl.groups = acl_group;
        }
        acl::ACL_TYPE_DEFAULT => {
            if user_obj_permissions != None
                || group_obj_permissions != None
                || other_permissions != None
                || mask_permissions != None
            {
                acl_default = Some(pxar_acl::Default {
                    // The value is set to UINT64_MAX as placeholder if one
                    // of the permissions is not set
                    user_obj_permissions: user_obj_permissions.unwrap_or(Permissions::NO_MASK),
                    group_obj_permissions: group_obj_permissions.unwrap_or(Permissions::NO_MASK),
                    other_permissions: other_permissions.unwrap_or(Permissions::NO_MASK),
                    mask_permissions: mask_permissions.unwrap_or(Permissions::NO_MASK),
                });
            }

            metadata.acl.default_users = acl_user;
            metadata.acl.default_groups = acl_group;
        }
        _ => bail!("Unexpected ACL type encountered"),
    }

    metadata.acl.group_obj = acl_group_obj;
    metadata.acl.default = acl_default;

    Ok(())
}

/// Note that our pattern lists are "positive". `MatchType::Include` means the file is included.
/// Since we are generating an *exclude* list, we need to invert this, so includes get a `'!'`
/// prefix.
fn generate_pxar_excludes_cli(patterns: &[MatchEntry]) -> Vec<u8> {
    use pathpatterns::{MatchFlag, MatchPattern};

    let mut content = Vec::new();

    for pattern in patterns {
        match pattern.match_type() {
            MatchType::Include => content.push(b'!'),
            MatchType::Exclude => (),
        }

        match pattern.pattern() {
            MatchPattern::Literal(lit) => content.extend(lit),
            MatchPattern::Pattern(pat) => content.extend(pat.pattern().to_bytes()),
        }

        if pattern.match_flags() == MatchFlag::MATCH_DIRECTORIES && content.last() != Some(&b'/') {
            content.push(b'/');
        }

        content.push(b'\n');
    }

    content
}
