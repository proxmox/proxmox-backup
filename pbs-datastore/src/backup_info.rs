use std::os::unix::io::RawFd;
use std::path::{Path, PathBuf};

use anyhow::{bail, Error};

use pbs_api_types::{BackupType, GroupFilter, BACKUP_DATE_REGEX, BACKUP_FILE_REGEX};

use super::manifest::MANIFEST_BLOB_NAME;

/// BackupGroup is a directory containing a list of BackupDir
#[derive(Debug, Hash, Clone)]
pub struct BackupGroup {
    group: pbs_api_types::BackupGroup,
}

impl BackupGroup {
    pub(crate) fn new<T: Into<String>>(backup_type: BackupType, backup_id: T) -> Self {
        Self {
            group: (backup_type, backup_id.into()).into(),
        }
    }

    /// Access the underlying [`BackupGroup`](pbs_api_types::BackupGroup).
    #[inline]
    pub fn group(&self) -> &pbs_api_types::BackupGroup {
        &self.group
    }

    pub fn backup_type(&self) -> BackupType {
        self.group.ty
    }

    pub fn backup_id(&self) -> &str {
        &self.group.id
    }

    pub fn relative_group_path(&self) -> PathBuf {
        self.group.to_string().into()
    }

    pub fn list_backups(&self, base_path: &Path) -> Result<Vec<BackupInfo>, Error> {
        let mut list = vec![];

        let mut path = base_path.to_owned();
        path.push(self.relative_group_path());

        proxmox_sys::fs::scandir(
            libc::AT_FDCWD,
            &path,
            &BACKUP_DATE_REGEX,
            |l2_fd, backup_time, file_type| {
                if file_type != nix::dir::Type::Directory {
                    return Ok(());
                }

                let backup_dir = self.backup_dir_with_rfc3339(backup_time)?;
                let files = list_backup_files(l2_fd, backup_time)?;

                let protected = backup_dir.is_protected(base_path.to_owned());

                list.push(BackupInfo {
                    backup_dir,
                    files,
                    protected,
                });

                Ok(())
            },
        )?;
        Ok(list)
    }

    /// Finds the latest backup inside a backup group
    pub fn last_backup(
        &self,
        base_path: &Path,
        only_finished: bool,
    ) -> Result<Option<BackupInfo>, Error> {
        let backups = self.list_backups(base_path)?;
        Ok(backups
            .into_iter()
            .filter(|item| !only_finished || item.is_finished())
            .max_by_key(|item| item.backup_dir.backup_time()))
    }

    pub fn last_successful_backup(&self, base_path: &Path) -> Result<Option<i64>, Error> {
        let mut last = None;

        let mut path = base_path.to_owned();
        path.push(self.relative_group_path());

        proxmox_sys::fs::scandir(
            libc::AT_FDCWD,
            &path,
            &BACKUP_DATE_REGEX,
            |l2_fd, backup_time, file_type| {
                if file_type != nix::dir::Type::Directory {
                    return Ok(());
                }

                let mut manifest_path = PathBuf::from(backup_time);
                manifest_path.push(MANIFEST_BLOB_NAME);

                use nix::fcntl::{openat, OFlag};
                match openat(
                    l2_fd,
                    &manifest_path,
                    OFlag::O_RDONLY,
                    nix::sys::stat::Mode::empty(),
                ) {
                    Ok(rawfd) => {
                        /* manifest exists --> assume backup was successful */
                        /* close else this leaks! */
                        nix::unistd::close(rawfd)?;
                    }
                    Err(nix::Error::Sys(nix::errno::Errno::ENOENT)) => {
                        return Ok(());
                    }
                    Err(err) => {
                        bail!("last_successful_backup: unexpected error - {}", err);
                    }
                }

                let timestamp = proxmox_time::parse_rfc3339(backup_time)?;
                if let Some(last_timestamp) = last {
                    if timestamp > last_timestamp {
                        last = Some(timestamp);
                    }
                } else {
                    last = Some(timestamp);
                }

                Ok(())
            },
        )?;

        Ok(last)
    }

    pub fn matches(&self, filter: &GroupFilter) -> bool {
        self.group.matches(filter)
    }

    pub fn backup_dir(&self, time: i64) -> Result<BackupDir, Error> {
        BackupDir::with_group(self.clone(), time)
    }

    pub fn backup_dir_with_rfc3339<T: Into<String>>(
        &self,
        time_string: T,
    ) -> Result<BackupDir, Error> {
        BackupDir::with_rfc3339(self.clone(), time_string.into())
    }
}

impl AsRef<pbs_api_types::BackupGroup> for BackupGroup {
    #[inline]
    fn as_ref(&self) -> &pbs_api_types::BackupGroup {
        &self.group
    }
}

impl From<&BackupGroup> for pbs_api_types::BackupGroup {
    fn from(group: &BackupGroup) -> pbs_api_types::BackupGroup {
        group.group.clone()
    }
}

impl From<BackupGroup> for pbs_api_types::BackupGroup {
    fn from(group: BackupGroup) -> pbs_api_types::BackupGroup {
        group.group
    }
}

impl std::fmt::Display for BackupGroup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let backup_type = self.backup_type();
        let id = self.backup_id();
        write!(f, "{}/{}", backup_type, id)
    }
}

impl From<BackupDir> for BackupGroup {
    fn from(dir: BackupDir) -> BackupGroup {
        BackupGroup {
            group: dir.dir.group,
        }
    }
}

impl From<&BackupDir> for BackupGroup {
    fn from(dir: &BackupDir) -> BackupGroup {
        BackupGroup {
            group: dir.dir.group.clone(),
        }
    }
}

/// Uniquely identify a Backup (relative to data store)
///
/// We also call this a backup snaphost.
#[derive(Debug, Clone)]
pub struct BackupDir {
    dir: pbs_api_types::BackupDir,
    // backup_time as rfc3339
    backup_time_string: String,
}

impl BackupDir {
    /// Temporarily used for tests.
    #[doc(hidden)]
    pub fn new_test(dir: pbs_api_types::BackupDir) -> Self {
        Self {
            backup_time_string: Self::backup_time_to_string(dir.time).unwrap(),
            dir,
        }
    }

    pub(crate) fn with_group(group: BackupGroup, backup_time: i64) -> Result<Self, Error> {
        let backup_time_string = Self::backup_time_to_string(backup_time)?;
        Ok(Self {
            dir: (group.group, backup_time).into(),
            backup_time_string,
        })
    }

    pub(crate) fn with_rfc3339(
        group: BackupGroup,
        backup_time_string: String,
    ) -> Result<Self, Error> {
        let backup_time = proxmox_time::parse_rfc3339(&backup_time_string)?;
        Ok(Self {
            dir: (group.group, backup_time).into(),
            backup_time_string,
        })
    }

    #[inline]
    pub fn backup_type(&self) -> BackupType {
        self.dir.group.ty
    }

    #[inline]
    pub fn backup_id(&self) -> &str {
        &self.dir.group.id
    }

    #[inline]
    pub fn backup_time(&self) -> i64 {
        self.dir.time
    }

    pub fn backup_time_string(&self) -> &str {
        &self.backup_time_string
    }

    pub fn relative_path(&self) -> PathBuf {
        format!("{}/{}", self.dir.group, self.backup_time_string).into()
    }

    /// Returns the absolute path for backup_dir, using the cached formatted time string.
    pub fn full_path(&self, mut base_path: PathBuf) -> PathBuf {
        base_path.push(self.dir.group.ty.as_str());
        base_path.push(&self.dir.group.id);
        base_path.push(&self.backup_time_string);
        base_path
    }

    pub fn protected_file(&self, mut path: PathBuf) -> PathBuf {
        path.push(self.relative_path());
        path.push(".protected");
        path
    }

    pub fn is_protected(&self, base_path: PathBuf) -> bool {
        let path = self.protected_file(base_path);
        path.exists()
    }

    pub fn backup_time_to_string(backup_time: i64) -> Result<String, Error> {
        // fixme: can this fail? (avoid unwrap)
        proxmox_time::epoch_to_rfc3339_utc(backup_time)
    }
}

impl AsRef<pbs_api_types::BackupDir> for BackupDir {
    fn as_ref(&self) -> &pbs_api_types::BackupDir {
        &self.dir
    }
}

impl AsRef<pbs_api_types::BackupGroup> for BackupDir {
    fn as_ref(&self) -> &pbs_api_types::BackupGroup {
        &self.dir.group
    }
}

impl From<&BackupDir> for pbs_api_types::BackupGroup {
    fn from(dir: &BackupDir) -> pbs_api_types::BackupGroup {
        dir.dir.group.clone()
    }
}

impl From<BackupDir> for pbs_api_types::BackupGroup {
    fn from(dir: BackupDir) -> pbs_api_types::BackupGroup {
        dir.dir.group.into()
    }
}

impl From<&BackupDir> for pbs_api_types::BackupDir {
    fn from(dir: &BackupDir) -> pbs_api_types::BackupDir {
        dir.dir.clone()
    }
}

impl From<BackupDir> for pbs_api_types::BackupDir {
    fn from(dir: BackupDir) -> pbs_api_types::BackupDir {
        dir.dir
    }
}

impl std::fmt::Display for BackupDir {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.dir.group, self.backup_time_string)
    }
}

/// Detailed Backup Information, lists files inside a BackupDir
#[derive(Debug, Clone)]
pub struct BackupInfo {
    /// the backup directory
    pub backup_dir: BackupDir,
    /// List of data files
    pub files: Vec<String>,
    /// Protection Status
    pub protected: bool,
}

impl BackupInfo {
    pub fn new(base_path: &Path, backup_dir: BackupDir) -> Result<BackupInfo, Error> {
        let mut path = base_path.to_owned();
        path.push(backup_dir.relative_path());

        let files = list_backup_files(libc::AT_FDCWD, &path)?;
        let protected = backup_dir.is_protected(base_path.to_owned());

        Ok(BackupInfo {
            backup_dir,
            files,
            protected,
        })
    }

    pub fn sort_list(list: &mut Vec<BackupInfo>, ascendending: bool) {
        if ascendending {
            // oldest first
            list.sort_unstable_by(|a, b| a.backup_dir.dir.time.cmp(&b.backup_dir.dir.time));
        } else {
            // newest first
            list.sort_unstable_by(|a, b| b.backup_dir.dir.time.cmp(&a.backup_dir.dir.time));
        }
    }

    pub fn is_finished(&self) -> bool {
        // backup is considered unfinished if there is no manifest
        self.files.iter().any(|name| name == MANIFEST_BLOB_NAME)
    }
}

fn list_backup_files<P: ?Sized + nix::NixPath>(
    dirfd: RawFd,
    path: &P,
) -> Result<Vec<String>, Error> {
    let mut files = vec![];

    proxmox_sys::fs::scandir(dirfd, path, &BACKUP_FILE_REGEX, |_, filename, file_type| {
        if file_type != nix::dir::Type::File {
            return Ok(());
        }
        files.push(filename.to_owned());
        Ok(())
    })?;

    Ok(files)
}
