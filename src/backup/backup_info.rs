use crate::tools;

use anyhow::{bail, format_err, Error};
use std::os::unix::io::RawFd;

use std::path::{Path, PathBuf};

use proxmox::const_regex;

use super::manifest::MANIFEST_BLOB_NAME;

macro_rules! BACKUP_ID_RE {
    () => {
        r"[A-Za-z0-9_][A-Za-z0-9._\-]*"
    };
}
macro_rules! BACKUP_TYPE_RE {
    () => {
        r"(?:host|vm|ct)"
    };
}
macro_rules! BACKUP_TIME_RE {
    () => {
        r"[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z"
    };
}

const_regex! {
    BACKUP_FILE_REGEX = r"^.*\.([fd]idx|blob)$";

    BACKUP_TYPE_REGEX = concat!(r"^(", BACKUP_TYPE_RE!(), r")$");

    pub BACKUP_ID_REGEX = concat!(r"^", BACKUP_ID_RE!(), r"$");

    BACKUP_DATE_REGEX = concat!(r"^", BACKUP_TIME_RE!() ,r"$");

    GROUP_PATH_REGEX = concat!(r"^(", BACKUP_TYPE_RE!(), ")/(", BACKUP_ID_RE!(), r")$");

    SNAPSHOT_PATH_REGEX = concat!(
        r"^(", BACKUP_TYPE_RE!(), ")/(", BACKUP_ID_RE!(), ")/(", BACKUP_TIME_RE!(), r")$");
}

/// BackupGroup is a directory containing a list of BackupDir
#[derive(Debug, Eq, PartialEq, Hash, Clone)]
pub struct BackupGroup {
    /// Type of backup
    backup_type: String,
    /// Unique (for this type) ID
    backup_id: String,
}

impl std::cmp::Ord for BackupGroup {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        let type_order = self.backup_type.cmp(&other.backup_type);
        if type_order != std::cmp::Ordering::Equal {
            return type_order;
        }
        // try to compare IDs numerically
        let id_self = self.backup_id.parse::<u64>();
        let id_other = other.backup_id.parse::<u64>();
        match (id_self, id_other) {
            (Ok(id_self), Ok(id_other)) => id_self.cmp(&id_other),
            (Ok(_), Err(_)) => std::cmp::Ordering::Less,
            (Err(_), Ok(_)) => std::cmp::Ordering::Greater,
            _ => self.backup_id.cmp(&other.backup_id),
        }
    }
}

impl std::cmp::PartialOrd for BackupGroup {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl BackupGroup {
    pub fn new<T: Into<String>, U: Into<String>>(backup_type: T, backup_id: U) -> Self {
        Self {
            backup_type: backup_type.into(),
            backup_id: backup_id.into(),
        }
    }

    pub fn backup_type(&self) -> &str {
        &self.backup_type
    }

    pub fn backup_id(&self) -> &str {
        &self.backup_id
    }

    pub fn group_path(&self) -> PathBuf {
        let mut relative_path = PathBuf::new();

        relative_path.push(&self.backup_type);

        relative_path.push(&self.backup_id);

        relative_path
    }

    pub fn list_backups(&self, base_path: &Path) -> Result<Vec<BackupInfo>, Error> {
        let mut list = vec![];

        let mut path = base_path.to_owned();
        path.push(self.group_path());

        tools::scandir(
            libc::AT_FDCWD,
            &path,
            &BACKUP_DATE_REGEX,
            |l2_fd, backup_time, file_type| {
                if file_type != nix::dir::Type::Directory {
                    return Ok(());
                }

                let backup_dir =
                    BackupDir::with_rfc3339(&self.backup_type, &self.backup_id, backup_time)?;
                let files = list_backup_files(l2_fd, backup_time)?;

                list.push(BackupInfo { backup_dir, files });

                Ok(())
            },
        )?;
        Ok(list)
    }

    pub fn last_successful_backup(&self, base_path: &Path) -> Result<Option<i64>, Error> {
        let mut last = None;

        let mut path = base_path.to_owned();
        path.push(self.group_path());

        tools::scandir(
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

                let timestamp = proxmox::tools::time::parse_rfc3339(backup_time)?;
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
}

impl std::fmt::Display for BackupGroup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let backup_type = self.backup_type();
        let id = self.backup_id();
        write!(f, "{}/{}", backup_type, id)
    }
}

impl std::str::FromStr for BackupGroup {
    type Err = Error;

    /// Parse a backup group path
    ///
    /// This parses strings like `vm/100".
    fn from_str(path: &str) -> Result<Self, Self::Err> {
        let cap = GROUP_PATH_REGEX
            .captures(path)
            .ok_or_else(|| format_err!("unable to parse backup group path '{}'", path))?;

        Ok(Self {
            backup_type: cap.get(1).unwrap().as_str().to_owned(),
            backup_id: cap.get(2).unwrap().as_str().to_owned(),
        })
    }
}

/// Uniquely identify a Backup (relative to data store)
///
/// We also call this a backup snaphost.
#[derive(Debug, Eq, PartialEq, Clone)]
pub struct BackupDir {
    /// Backup group
    group: BackupGroup,
    /// Backup timestamp
    backup_time: i64,
    // backup_time as rfc3339
    backup_time_string: String,
}

impl BackupDir {
    pub fn new<T, U>(backup_type: T, backup_id: U, backup_time: i64) -> Result<Self, Error>
    where
        T: Into<String>,
        U: Into<String>,
    {
        let group = BackupGroup::new(backup_type.into(), backup_id.into());
        BackupDir::with_group(group, backup_time)
    }

    pub fn with_rfc3339<T, U, V>(
        backup_type: T,
        backup_id: U,
        backup_time_string: V,
    ) -> Result<Self, Error>
    where
        T: Into<String>,
        U: Into<String>,
        V: Into<String>,
    {
        let backup_time_string = backup_time_string.into();
        let backup_time = proxmox::tools::time::parse_rfc3339(&backup_time_string)?;
        let group = BackupGroup::new(backup_type.into(), backup_id.into());
        Ok(Self {
            group,
            backup_time,
            backup_time_string,
        })
    }

    pub fn with_group(group: BackupGroup, backup_time: i64) -> Result<Self, Error> {
        let backup_time_string = Self::backup_time_to_string(backup_time)?;
        Ok(Self {
            group,
            backup_time,
            backup_time_string,
        })
    }

    pub fn group(&self) -> &BackupGroup {
        &self.group
    }

    pub fn backup_time(&self) -> i64 {
        self.backup_time
    }

    pub fn backup_time_string(&self) -> &str {
        &self.backup_time_string
    }

    pub fn relative_path(&self) -> PathBuf {
        let mut relative_path = self.group.group_path();

        relative_path.push(self.backup_time_string.clone());

        relative_path
    }

    pub fn backup_time_to_string(backup_time: i64) -> Result<String, Error> {
        // fixme: can this fail? (avoid unwrap)
        proxmox::tools::time::epoch_to_rfc3339_utc(backup_time)
    }
}

impl std::str::FromStr for BackupDir {
    type Err = Error;

    /// Parse a snapshot path
    ///
    /// This parses strings like `host/elsa/2020-06-15T05:18:33Z".
    fn from_str(path: &str) -> Result<Self, Self::Err> {
        let cap = SNAPSHOT_PATH_REGEX
            .captures(path)
            .ok_or_else(|| format_err!("unable to parse backup snapshot path '{}'", path))?;

        BackupDir::with_rfc3339(
            cap.get(1).unwrap().as_str(),
            cap.get(2).unwrap().as_str(),
            cap.get(3).unwrap().as_str(),
        )
    }
}

impl std::fmt::Display for BackupDir {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let backup_type = self.group.backup_type();
        let id = self.group.backup_id();
        write!(f, "{}/{}/{}", backup_type, id, self.backup_time_string)
    }
}

/// Detailed Backup Information, lists files inside a BackupDir
#[derive(Debug, Clone)]
pub struct BackupInfo {
    /// the backup directory
    pub backup_dir: BackupDir,
    /// List of data files
    pub files: Vec<String>,
}

impl BackupInfo {
    pub fn new(base_path: &Path, backup_dir: BackupDir) -> Result<BackupInfo, Error> {
        let mut path = base_path.to_owned();
        path.push(backup_dir.relative_path());

        let files = list_backup_files(libc::AT_FDCWD, &path)?;

        Ok(BackupInfo { backup_dir, files })
    }

    /// Finds the latest backup inside a backup group
    pub fn last_backup(
        base_path: &Path,
        group: &BackupGroup,
        only_finished: bool,
    ) -> Result<Option<BackupInfo>, Error> {
        let backups = group.list_backups(base_path)?;
        Ok(backups
            .into_iter()
            .filter(|item| !only_finished || item.is_finished())
            .max_by_key(|item| item.backup_dir.backup_time()))
    }

    pub fn sort_list(list: &mut Vec<BackupInfo>, ascendending: bool) {
        if ascendending {
            // oldest first
            list.sort_unstable_by(|a, b| a.backup_dir.backup_time.cmp(&b.backup_dir.backup_time));
        } else {
            // newest first
            list.sort_unstable_by(|a, b| b.backup_dir.backup_time.cmp(&a.backup_dir.backup_time));
        }
    }

    pub fn list_files(base_path: &Path, backup_dir: &BackupDir) -> Result<Vec<String>, Error> {
        let mut path = base_path.to_owned();
        path.push(backup_dir.relative_path());

        let files = list_backup_files(libc::AT_FDCWD, &path)?;

        Ok(files)
    }

    pub fn list_backup_groups(base_path: &Path) -> Result<Vec<BackupGroup>, Error> {
        let mut list = Vec::new();

        tools::scandir(
            libc::AT_FDCWD,
            base_path,
            &BACKUP_TYPE_REGEX,
            |l0_fd, backup_type, file_type| {
                if file_type != nix::dir::Type::Directory {
                    return Ok(());
                }
                tools::scandir(
                    l0_fd,
                    backup_type,
                    &BACKUP_ID_REGEX,
                    |_, backup_id, file_type| {
                        if file_type != nix::dir::Type::Directory {
                            return Ok(());
                        }

                        list.push(BackupGroup::new(backup_type, backup_id));

                        Ok(())
                    },
                )
            },
        )?;

        Ok(list)
    }

    pub fn is_finished(&self) -> bool {
        // backup is considered unfinished if there is no manifest
        self.files
            .iter()
            .any(|name| name == super::MANIFEST_BLOB_NAME)
    }
}

fn list_backup_files<P: ?Sized + nix::NixPath>(
    dirfd: RawFd,
    path: &P,
) -> Result<Vec<String>, Error> {
    let mut files = vec![];

    tools::scandir(dirfd, path, &BACKUP_FILE_REGEX, |_, filename, file_type| {
        if file_type != nix::dir::Type::File {
            return Ok(());
        }
        files.push(filename.to_owned());
        Ok(())
    })?;

    Ok(files)
}
