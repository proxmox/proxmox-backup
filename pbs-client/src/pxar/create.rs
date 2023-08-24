use std::collections::{HashMap, HashSet};
use std::ffi::{CStr, CString, OsStr};
use std::fmt;
use std::io::{self, Read};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd, RawFd};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{bail, Context, Error};
use futures::future::BoxFuture;
use futures::FutureExt;
use nix::dir::Dir;
use nix::errno::Errno;
use nix::fcntl::OFlag;
use nix::sys::stat::{FileStat, Mode};

use pathpatterns::{MatchEntry, MatchFlag, MatchList, MatchType, PatternFlag};
use proxmox_sys::error::SysError;
use pxar::encoder::{LinkOffset, SeqWrite};
use pxar::Metadata;

use proxmox_io::vec;
use proxmox_lang::c_str;
use proxmox_sys::fs::{self, acl, xattr};

use pbs_datastore::catalog::BackupCatalogWriter;

use crate::pxar::metadata::errno_is_unsupported;
use crate::pxar::tools::assert_single_path_component;
use crate::pxar::Flags;

/// Pxar options for creating a pxar archive/stream
#[derive(Default, Clone)]
pub struct PxarCreateOptions {
    /// Device/mountpoint st_dev numbers that should be included. None for no limitation.
    pub device_set: Option<HashSet<u64>>,
    /// Exclusion patterns
    pub patterns: Vec<MatchEntry>,
    /// Maximum number of entries to hold in memory
    pub entries_max: usize,
    /// Skip lost+found directory
    pub skip_lost_and_found: bool,
}

fn detect_fs_type(fd: RawFd) -> Result<i64, Error> {
    let mut fs_stat = std::mem::MaybeUninit::uninit();
    let res = unsafe { libc::fstatfs(fd, fs_stat.as_mut_ptr()) };
    Errno::result(res)?;
    let fs_stat = unsafe { fs_stat.assume_init() };

    Ok(fs_stat.f_type)
}

fn strip_ascii_whitespace(line: &[u8]) -> &[u8] {
    let line = match line.iter().position(|&b| !b.is_ascii_whitespace()) {
        Some(n) => &line[n..],
        None => return &[],
    };
    match line.iter().rev().position(|&b| !b.is_ascii_whitespace()) {
        Some(n) => &line[..(line.len() - n)],
        None => &[],
    }
}

#[rustfmt::skip]
pub fn is_virtual_file_system(magic: i64) -> bool {
    use proxmox_sys::linux::magic::*;

    matches!(magic, BINFMTFS_MAGIC |
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
        SYSFS_MAGIC)
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

struct Archiver {
    feature_flags: Flags,
    fs_feature_flags: Flags,
    fs_magic: i64,
    patterns: Vec<MatchEntry>,
    #[allow(clippy::type_complexity)]
    callback: Box<dyn FnMut(&Path) -> Result<(), Error> + Send>,
    catalog: Option<Arc<Mutex<dyn BackupCatalogWriter + Send>>>,
    path: PathBuf,
    entry_counter: usize,
    entry_limit: usize,
    current_st_dev: libc::dev_t,
    device_set: Option<HashSet<u64>>,
    hardlinks: HashMap<HardLinkInfo, (PathBuf, LinkOffset)>,
    file_copy_buffer: Vec<u8>,
}

type Encoder<'a, T> = pxar::encoder::aio::Encoder<'a, T>;

pub async fn create_archive<T, F>(
    source_dir: Dir,
    mut writer: T,
    feature_flags: Flags,
    callback: F,
    catalog: Option<Arc<Mutex<dyn BackupCatalogWriter + Send>>>,
    options: PxarCreateOptions,
) -> Result<(), Error>
where
    T: SeqWrite + Send,
    F: FnMut(&Path) -> Result<(), Error> + Send + 'static,
{
    let fs_magic = detect_fs_type(source_dir.as_raw_fd())?;
    if is_virtual_file_system(fs_magic) {
        bail!("refusing to backup a virtual file system");
    }

    let mut fs_feature_flags = Flags::from_magic(fs_magic);

    let stat = nix::sys::stat::fstat(source_dir.as_raw_fd())?;
    let metadata = get_metadata(
        source_dir.as_raw_fd(),
        &stat,
        feature_flags & fs_feature_flags,
        fs_magic,
        &mut fs_feature_flags,
    )
    .context("failed to get metadata for source directory")?;

    let mut device_set = options.device_set.clone();
    if let Some(ref mut set) = device_set {
        set.insert(stat.st_dev);
    }

    let mut encoder = Encoder::new(&mut writer, &metadata).await?;

    let mut patterns = options.patterns;

    if options.skip_lost_and_found {
        patterns.push(MatchEntry::parse_pattern(
            "lost+found",
            PatternFlag::PATH_NAME,
            MatchType::Exclude,
        )?);
    }

    let mut archiver = Archiver {
        feature_flags,
        fs_feature_flags,
        fs_magic,
        callback: Box::new(callback),
        patterns,
        catalog,
        path: PathBuf::new(),
        entry_counter: 0,
        entry_limit: options.entries_max,
        current_st_dev: stat.st_dev,
        device_set,
        hardlinks: HashMap::new(),
        file_copy_buffer: vec::undefined(4 * 1024 * 1024),
    };

    archiver
        .archive_dir_contents(&mut encoder, source_dir, true)
        .await?;
    encoder.finish().await?;
    Ok(())
}

struct FileListEntry {
    name: CString,
    path: PathBuf,
    stat: FileStat,
}

impl Archiver {
    /// Get the currently effective feature flags. (Requested flags masked by the file system
    /// feature flags).
    fn flags(&self) -> Flags {
        self.feature_flags & self.fs_feature_flags
    }

    fn wrap_err(&self, err: Error) -> Error {
        if err.downcast_ref::<ArchiveError>().is_some() {
            err
        } else {
            ArchiveError::new(self.path.clone(), err).into()
        }
    }

    fn archive_dir_contents<'a, 'b, T: SeqWrite + Send>(
        &'a mut self,
        encoder: &'a mut Encoder<'b, T>,
        mut dir: Dir,
        is_root: bool,
    ) -> BoxFuture<'a, Result<(), Error>> {
        async move {
            let entry_counter = self.entry_counter;

            let old_patterns_count = self.patterns.len();
            self.read_pxar_excludes(dir.as_raw_fd())?;

            let mut file_list = self.generate_directory_file_list(&mut dir, is_root)?;

            if is_root && old_patterns_count > 0 {
                file_list.push(FileListEntry {
                    name: CString::new(".pxarexclude-cli").unwrap(),
                    path: PathBuf::new(),
                    stat: unsafe { std::mem::zeroed() },
                });
            }

            let dir_fd = dir.as_raw_fd();

            let old_path = std::mem::take(&mut self.path);

            for file_entry in file_list {
                let file_name = file_entry.name.to_bytes();

                if is_root && file_name == b".pxarexclude-cli" {
                    self.encode_pxarexclude_cli(encoder, &file_entry.name, old_patterns_count)
                        .await?;
                    continue;
                }

                (self.callback)(&file_entry.path)?;
                self.path = file_entry.path;
                self.add_entry(encoder, dir_fd, &file_entry.name, &file_entry.stat)
                    .await
                    .map_err(|err| self.wrap_err(err))?;
            }
            self.path = old_path;
            self.entry_counter = entry_counter;
            self.patterns.truncate(old_patterns_count);

            Ok(())
        }
        .boxed()
    }

    /// openat() wrapper which allows but logs `EACCES` and turns `ENOENT` into `None`.
    ///
    /// The `existed` flag is set when iterating through a directory to note that we know the file
    /// is supposed to exist and we should warn if it doesnt'.
    fn open_file(
        &mut self,
        parent: RawFd,
        file_name: &CStr,
        oflags: OFlag,
        existed: bool,
    ) -> Result<Option<OwnedFd>, Error> {
        // common flags we always want to use:
        let oflags = oflags | OFlag::O_CLOEXEC | OFlag::O_NOCTTY;

        let mut noatime = OFlag::O_NOATIME;
        loop {
            return match proxmox_sys::fd::openat(
                &parent,
                file_name,
                oflags | noatime,
                Mode::empty(),
            ) {
                Ok(fd) => Ok(Some(fd)),
                Err(Errno::ENOENT) => {
                    if existed {
                        self.report_vanished_file()?;
                    }
                    Ok(None)
                }
                Err(Errno::EACCES) => {
                    log::warn!("failed to open file: {:?}: access denied", file_name);
                    Ok(None)
                }
                Err(Errno::EPERM) if !noatime.is_empty() => {
                    // Retry without O_NOATIME:
                    noatime = OFlag::empty();
                    continue;
                }
                Err(other) => Err(Error::from(other)),
            };
        }
    }

    fn read_pxar_excludes(&mut self, parent: RawFd) -> Result<(), Error> {
        let fd = match self.open_file(parent, c_str!(".pxarexclude"), OFlag::O_RDONLY, false)? {
            Some(fd) => fd,
            None => return Ok(()),
        };

        let old_pattern_count = self.patterns.len();

        let path_bytes = self.path.as_os_str().as_bytes();

        let file = unsafe { std::fs::File::from_raw_fd(fd.into_raw_fd()) };

        use io::BufRead;
        for line in io::BufReader::new(file).split(b'\n') {
            let line = match line {
                Ok(line) => line,
                Err(err) => {
                    log::warn!(
                        "ignoring .pxarexclude after read error in {:?}: {}",
                        self.path,
                        err,
                    );
                    self.patterns.truncate(old_pattern_count);
                    return Ok(());
                }
            };

            let line = strip_ascii_whitespace(&line);

            if line.is_empty() || line[0] == b'#' {
                continue;
            }

            let mut buf;
            let (line, mode, anchored) = if line[0] == b'/' {
                buf = Vec::with_capacity(path_bytes.len() + 1 + line.len());
                buf.extend(path_bytes);
                buf.extend(line);
                (&buf[..], MatchType::Exclude, true)
            } else if line.starts_with(b"!/") {
                // inverted case with absolute path
                buf = Vec::with_capacity(path_bytes.len() + line.len());
                buf.extend(path_bytes);
                buf.extend(&line[1..]); // without the '!'
                (&buf[..], MatchType::Include, true)
            } else if line.starts_with(b"!") {
                (&line[1..], MatchType::Include, false)
            } else {
                (line, MatchType::Exclude, false)
            };

            match MatchEntry::parse_pattern(line, PatternFlag::PATH_NAME, mode) {
                Ok(pattern) => {
                    if anchored {
                        self.patterns.push(pattern.add_flags(MatchFlag::ANCHORED));
                    } else {
                        self.patterns.push(pattern);
                    }
                }
                Err(err) => {
                    log::error!("bad pattern in {:?}: {}", self.path, err);
                }
            }
        }

        Ok(())
    }

    async fn encode_pxarexclude_cli<T: SeqWrite + Send>(
        &mut self,
        encoder: &mut Encoder<'_, T>,
        file_name: &CStr,
        patterns_count: usize,
    ) -> Result<(), Error> {
        let content = generate_pxar_excludes_cli(&self.patterns[..patterns_count]);
        if let Some(ref catalog) = self.catalog {
            catalog
                .lock()
                .unwrap()
                .add_file(file_name, content.len() as u64, 0)?;
        }

        let mut metadata = Metadata::default();
        metadata.stat.mode = pxar::format::mode::IFREG | 0o600;

        let mut file = encoder
            .create_file(&metadata, ".pxarexclude-cli", content.len() as u64)
            .await?;
        file.write_all(&content).await?;

        Ok(())
    }

    fn generate_directory_file_list(
        &mut self,
        dir: &mut Dir,
        is_root: bool,
    ) -> Result<Vec<FileListEntry>, Error> {
        let dir_fd = dir.as_raw_fd();

        let mut file_list = Vec::new();

        for file in dir.iter() {
            let file = file?;

            let file_name = file.file_name();
            let file_name_bytes = file_name.to_bytes();
            if file_name_bytes == b"." || file_name_bytes == b".." {
                continue;
            }

            if is_root && file_name_bytes == b".pxarexclude-cli" {
                continue;
            }

            let os_file_name = OsStr::from_bytes(file_name_bytes);
            assert_single_path_component(os_file_name)?;
            let full_path = self.path.join(os_file_name);

            let match_path = PathBuf::from("/").join(full_path.clone());

            let mut stat_results: Option<FileStat> = None;

            let get_file_mode = || {
                nix::sys::stat::fstatat(dir_fd, file_name, nix::fcntl::AtFlags::AT_SYMLINK_NOFOLLOW)
            };

            let match_result = self
                .patterns
                .matches(match_path.as_os_str().as_bytes(), || {
                    Ok::<_, Errno>(match &stat_results {
                        Some(result) => result.st_mode,
                        None => stat_results.insert(get_file_mode()?).st_mode,
                    })
                });

            match match_result {
                Ok(Some(MatchType::Exclude)) => continue,
                Ok(_) => (),
                Err(err) if err.not_found() => continue,
                Err(err) => {
                    return Err(err).with_context(|| format!("stat failed on {full_path:?}"))
                }
            }

            let stat = stat_results
                .map(Ok)
                .unwrap_or_else(get_file_mode)
                .with_context(|| format!("stat failed on {full_path:?}"))?;

            self.entry_counter += 1;
            if self.entry_counter > self.entry_limit {
                bail!(
                    "exceeded allowed number of file entries (> {})",
                    self.entry_limit
                );
            }

            file_list.push(FileListEntry {
                name: file_name.to_owned(),
                path: full_path,
                stat,
            });
        }

        file_list.sort_unstable_by(|a, b| a.name.cmp(&b.name));

        Ok(file_list)
    }

    fn report_vanished_file(&mut self) -> Result<(), Error> {
        log::warn!("warning: file vanished while reading: {:?}", self.path);
        Ok(())
    }

    fn report_file_shrunk_while_reading(&mut self) -> Result<(), Error> {
        log::warn!(
            "warning: file size shrunk while reading: {:?}, file will be padded with zeros!",
            self.path,
        );
        Ok(())
    }

    fn report_file_grew_while_reading(&mut self) -> Result<(), Error> {
        log::warn!(
            "warning: file size increased while reading: {:?}, file will be truncated!",
            self.path,
        );
        Ok(())
    }

    async fn add_entry<T: SeqWrite + Send>(
        &mut self,
        encoder: &mut Encoder<'_, T>,
        parent: RawFd,
        c_file_name: &CStr,
        stat: &FileStat,
    ) -> Result<(), Error> {
        use pxar::format::mode;

        let file_mode = stat.st_mode & libc::S_IFMT;
        let open_mode = if file_mode == libc::S_IFREG || file_mode == libc::S_IFDIR {
            OFlag::empty()
        } else {
            OFlag::O_PATH
        };

        let fd = self.open_file(
            parent,
            c_file_name,
            open_mode | OFlag::O_RDONLY | OFlag::O_NOFOLLOW,
            true,
        )?;

        let fd = match fd {
            Some(fd) => fd,
            None => return Ok(()),
        };

        let metadata = get_metadata(
            fd.as_raw_fd(),
            stat,
            self.flags(),
            self.fs_magic,
            &mut self.fs_feature_flags,
        )?;

        let match_path = PathBuf::from("/").join(self.path.clone());
        if self
            .patterns
            .matches(match_path.as_os_str().as_bytes(), stat.st_mode)?
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
                        if let Some(ref catalog) = self.catalog {
                            catalog.lock().unwrap().add_hardlink(c_file_name)?;
                        }

                        encoder.add_hardlink(file_name, path, *offset).await?;

                        return Ok(());
                    }
                }

                let file_size = stat.st_size as u64;
                if let Some(ref catalog) = self.catalog {
                    catalog
                        .lock()
                        .unwrap()
                        .add_file(c_file_name, file_size, stat.st_mtime)?;
                }

                let offset: LinkOffset = self
                    .add_regular_file(encoder, fd, file_name, &metadata, file_size)
                    .await?;

                if stat.st_nlink > 1 {
                    self.hardlinks
                        .insert(link_info, (self.path.clone(), offset));
                }

                Ok(())
            }
            mode::IFDIR => {
                let dir = Dir::from_fd(fd.into_raw_fd())?;

                if let Some(ref catalog) = self.catalog {
                    catalog.lock().unwrap().start_directory(c_file_name)?;
                }
                let result = self
                    .add_directory(encoder, dir, c_file_name, &metadata, stat)
                    .await;
                if let Some(ref catalog) = self.catalog {
                    catalog.lock().unwrap().end_directory()?;
                }
                result
            }
            mode::IFSOCK => {
                if let Some(ref catalog) = self.catalog {
                    catalog.lock().unwrap().add_socket(c_file_name)?;
                }

                Ok(encoder.add_socket(&metadata, file_name).await?)
            }
            mode::IFIFO => {
                if let Some(ref catalog) = self.catalog {
                    catalog.lock().unwrap().add_fifo(c_file_name)?;
                }

                Ok(encoder.add_fifo(&metadata, file_name).await?)
            }
            mode::IFLNK => {
                if let Some(ref catalog) = self.catalog {
                    catalog.lock().unwrap().add_symlink(c_file_name)?;
                }

                self.add_symlink(encoder, fd, file_name, &metadata).await
            }
            mode::IFBLK => {
                if let Some(ref catalog) = self.catalog {
                    catalog.lock().unwrap().add_block_device(c_file_name)?;
                }

                self.add_device(encoder, file_name, &metadata, stat).await
            }
            mode::IFCHR => {
                if let Some(ref catalog) = self.catalog {
                    catalog.lock().unwrap().add_char_device(c_file_name)?;
                }

                self.add_device(encoder, file_name, &metadata, stat).await
            }
            other => bail!(
                "encountered unknown file type: 0x{:x} (0o{:o})",
                other,
                other
            ),
        }
    }

    async fn add_directory<T: SeqWrite + Send>(
        &mut self,
        encoder: &mut Encoder<'_, T>,
        dir: Dir,
        dir_name: &CStr,
        metadata: &Metadata,
        stat: &FileStat,
    ) -> Result<(), Error> {
        let dir_name = OsStr::from_bytes(dir_name.to_bytes());

        let mut encoder = encoder.create_directory(dir_name, metadata).await?;

        let old_fs_magic = self.fs_magic;
        let old_fs_feature_flags = self.fs_feature_flags;
        let old_st_dev = self.current_st_dev;

        let mut skip_contents = false;
        if old_st_dev != stat.st_dev {
            self.fs_magic = detect_fs_type(dir.as_raw_fd())?;
            self.fs_feature_flags = Flags::from_magic(self.fs_magic);
            self.current_st_dev = stat.st_dev;

            if is_virtual_file_system(self.fs_magic) {
                skip_contents = true;
            } else if let Some(set) = &self.device_set {
                skip_contents = !set.contains(&stat.st_dev);
            }
        }

        let result = if skip_contents {
            log::info!("skipping mount point: {:?}", self.path);
            Ok(())
        } else {
            self.archive_dir_contents(&mut encoder, dir, false).await
        };

        self.fs_magic = old_fs_magic;
        self.fs_feature_flags = old_fs_feature_flags;
        self.current_st_dev = old_st_dev;

        encoder.finish().await?;
        result
    }

    async fn add_regular_file<T: SeqWrite + Send>(
        &mut self,
        encoder: &mut Encoder<'_, T>,
        fd: OwnedFd,
        file_name: &Path,
        metadata: &Metadata,
        file_size: u64,
    ) -> Result<LinkOffset, Error> {
        let mut file = unsafe { std::fs::File::from_raw_fd(fd.into_raw_fd()) };
        let mut remaining = file_size;
        let mut out = encoder.create_file(metadata, file_name, file_size).await?;
        while remaining != 0 {
            let mut got = match file.read(&mut self.file_copy_buffer[..]) {
                Ok(0) => break,
                Ok(got) => got,
                Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(err) => bail!(err),
            };
            if got as u64 > remaining {
                self.report_file_grew_while_reading()?;
                got = remaining as usize;
            }
            out.write_all(&self.file_copy_buffer[..got]).await?;
            remaining -= got as u64;
        }
        if remaining > 0 {
            self.report_file_shrunk_while_reading()?;
            let to_zero = remaining.min(self.file_copy_buffer.len() as u64) as usize;
            vec::clear(&mut self.file_copy_buffer[..to_zero]);
            while remaining != 0 {
                let fill = remaining.min(self.file_copy_buffer.len() as u64) as usize;
                out.write_all(&self.file_copy_buffer[..fill]).await?;
                remaining -= fill as u64;
            }
        }

        Ok(out.file_offset())
    }

    async fn add_symlink<T: SeqWrite + Send>(
        &mut self,
        encoder: &mut Encoder<'_, T>,
        fd: OwnedFd,
        file_name: &Path,
        metadata: &Metadata,
    ) -> Result<(), Error> {
        let dest = nix::fcntl::readlinkat(fd.as_raw_fd(), &b""[..])?;
        encoder.add_symlink(metadata, file_name, dest).await?;
        Ok(())
    }

    async fn add_device<T: SeqWrite + Send>(
        &mut self,
        encoder: &mut Encoder<'_, T>,
        file_name: &Path,
        metadata: &Metadata,
        stat: &FileStat,
    ) -> Result<(), Error> {
        Ok(encoder
            .add_device(
                metadata,
                file_name,
                pxar::format::Device::from_dev_t(stat.st_rdev),
            )
            .await?)
    }
}

fn get_metadata(
    fd: RawFd,
    stat: &FileStat,
    flags: Flags,
    fs_magic: i64,
    fs_feature_flags: &mut Flags,
) -> Result<Metadata, Error> {
    // required for some of these
    let proc_path = Path::new("/proc/self/fd/").join(fd.to_string());

    let mut meta = Metadata {
        stat: pxar::Stat {
            mode: u64::from(stat.st_mode),
            flags: 0,
            uid: stat.st_uid,
            gid: stat.st_gid,
            mtime: pxar::format::StatxTimestamp::new(stat.st_mtime, stat.st_mtime_nsec as u32),
        },
        ..Default::default()
    };

    get_xattr_fcaps_acl(&mut meta, fd, &proc_path, flags, fs_feature_flags)?;
    get_chattr(&mut meta, fd)?;
    get_fat_attr(&mut meta, fd, fs_magic)?;
    get_quota_project_id(&mut meta, fd, flags, fs_magic)?;
    Ok(meta)
}

fn get_fcaps(
    meta: &mut Metadata,
    fd: RawFd,
    flags: Flags,
    fs_feature_flags: &mut Flags,
) -> Result<(), Error> {
    if !flags.contains(Flags::WITH_FCAPS) {
        return Ok(());
    }

    match xattr::fgetxattr(fd, xattr::xattr_name_fcaps()) {
        Ok(data) => {
            meta.fcaps = Some(pxar::format::FCaps { data });
            Ok(())
        }
        Err(Errno::ENODATA) => Ok(()),
        Err(Errno::EOPNOTSUPP) => {
            fs_feature_flags.remove(Flags::WITH_FCAPS);
            Ok(())
        }
        Err(Errno::EBADF) => Ok(()), // symlinks
        Err(err) => Err(err).context("failed to read file capabilities"),
    }
}

fn get_xattr_fcaps_acl(
    meta: &mut Metadata,
    fd: RawFd,
    proc_path: &Path,
    flags: Flags,
    fs_feature_flags: &mut Flags,
) -> Result<(), Error> {
    if !flags.contains(Flags::WITH_XATTRS) {
        return Ok(());
    }

    let xattrs = match xattr::flistxattr(fd) {
        Ok(names) => names,
        Err(Errno::EOPNOTSUPP) => {
            fs_feature_flags.remove(Flags::WITH_XATTRS);
            return Ok(());
        }
        Err(Errno::EBADF) => return Ok(()), // symlinks
        Err(err) => return Err(err).context("failed to read xattrs"),
    };

    for attr in &xattrs {
        if xattr::is_security_capability(attr) {
            get_fcaps(meta, fd, flags, fs_feature_flags)?;
            continue;
        }

        if xattr::is_acl(attr) {
            get_acl(meta, proc_path, flags, fs_feature_flags)?;
            continue;
        }

        if !xattr::is_valid_xattr_name(attr) {
            continue;
        }

        match xattr::fgetxattr(fd, attr) {
            Ok(data) => meta
                .xattrs
                .push(pxar::format::XAttr::new(attr.to_bytes(), data)),
            Err(Errno::ENODATA) => (), // it got removed while we were iterating...
            Err(Errno::EOPNOTSUPP) => (), // shouldn't be possible so just ignore this
            Err(Errno::EBADF) => (),   // symlinks, shouldn't be able to reach this either
            Err(err) => {
                return Err(err).context(format!("error reading extended attribute {attr:?}"))
            }
        }
    }

    Ok(())
}

fn get_chattr(metadata: &mut Metadata, fd: RawFd) -> Result<(), Error> {
    let mut attr: libc::c_long = 0;

    match unsafe { fs::read_attr_fd(fd, &mut attr) } {
        Ok(_) => (),
        Err(errno) if errno_is_unsupported(errno) => {
            return Ok(());
        }
        Err(err) => return Err(err).context("failed to read file attributes"),
    }

    metadata.stat.flags |= Flags::from_chattr(attr).bits();

    Ok(())
}

fn get_fat_attr(metadata: &mut Metadata, fd: RawFd, fs_magic: i64) -> Result<(), Error> {
    use proxmox_sys::linux::magic::*;

    if fs_magic != MSDOS_SUPER_MAGIC && fs_magic != FUSE_SUPER_MAGIC {
        return Ok(());
    }

    let mut attr: u32 = 0;

    match unsafe { fs::read_fat_attr_fd(fd, &mut attr) } {
        Ok(_) => (),
        Err(errno) if errno_is_unsupported(errno) => {
            return Ok(());
        }
        Err(err) => return Err(err).context("failed to read fat attributes"),
    }

    metadata.stat.flags |= Flags::from_fat_attr(attr).bits();

    Ok(())
}

/// Read the quota project id for an inode, supported on ext4/XFS/FUSE/ZFS filesystems
fn get_quota_project_id(
    metadata: &mut Metadata,
    fd: RawFd,
    flags: Flags,
    magic: i64,
) -> Result<(), Error> {
    if !(metadata.is_dir() || metadata.is_regular_file()) {
        return Ok(());
    }

    if !flags.contains(Flags::WITH_QUOTA_PROJID) {
        return Ok(());
    }

    use proxmox_sys::linux::magic::*;

    match magic {
        EXT4_SUPER_MAGIC | XFS_SUPER_MAGIC | FUSE_SUPER_MAGIC | ZFS_SUPER_MAGIC => (),
        _ => return Ok(()),
    }

    let mut fsxattr = fs::FSXAttr::default();
    let res = unsafe { fs::fs_ioc_fsgetxattr(fd, &mut fsxattr) };

    // On some FUSE filesystems it can happen that ioctl is not supported.
    // For these cases projid is set to 0 while the error is ignored.
    if let Err(errno) = res {
        if errno_is_unsupported(errno) {
            return Ok(());
        } else {
            return Err(errno).context("error while reading quota project id");
        }
    }

    let projid = fsxattr.fsx_projid as u64;
    if projid != 0 {
        metadata.quota_project_id = Some(pxar::format::QuotaProjectId { projid });
    }
    Ok(())
}

fn get_acl(
    metadata: &mut Metadata,
    proc_path: &Path,
    flags: Flags,
    fs_feature_flags: &mut Flags,
) -> Result<(), Error> {
    if !flags.contains(Flags::WITH_ACL) {
        return Ok(());
    }

    if metadata.is_symlink() {
        return Ok(());
    }

    get_acl_do(metadata, proc_path, acl::ACL_TYPE_ACCESS, fs_feature_flags)?;

    if metadata.is_dir() {
        get_acl_do(metadata, proc_path, acl::ACL_TYPE_DEFAULT, fs_feature_flags)?;
    }

    Ok(())
}

fn get_acl_do(
    metadata: &mut Metadata,
    proc_path: &Path,
    acl_type: acl::ACLType,
    fs_feature_flags: &mut Flags,
) -> Result<(), Error> {
    // In order to be able to get ACLs with type ACL_TYPE_DEFAULT, we have
    // to create a path for acl_get_file(). acl_get_fd() only allows to get
    // ACL_TYPE_ACCESS attributes.
    let acl = match acl::ACL::get_file(proc_path, acl_type) {
        Ok(acl) => acl,
        // Don't bail if underlying endpoint does not support acls
        Err(Errno::EOPNOTSUPP) => {
            fs_feature_flags.remove(Flags::WITH_ACL);
            return Ok(());
        }
        // Don't bail if the endpoint cannot carry acls
        Err(Errno::EBADF) => return Ok(()),
        // Don't bail if there is no data
        Err(Errno::ENODATA) => return Ok(()),
        Err(err) => return Err(err).context("error while reading ACL"),
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
            metadata.acl.group_obj = acl_group_obj;
        }
        acl::ACL_TYPE_DEFAULT => {
            if user_obj_permissions.is_some()
                || group_obj_permissions.is_some()
                || other_permissions.is_some()
                || mask_permissions.is_some()
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
            metadata.acl.default = acl_default;
        }
        _ => bail!("Unexpected ACL type encountered"),
    }

    Ok(())
}

/// Note that our pattern lists are "positive". `MatchType::Include` means the file is included.
/// Since we are generating an *exclude* list, we need to invert this, so includes get a `'!'`
/// prefix.
fn generate_pxar_excludes_cli(patterns: &[MatchEntry]) -> Vec<u8> {
    use pathpatterns::MatchPattern;

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
