use crate::tools;

use anyhow::{bail, format_err, Error};
use regex::Regex;
use std::os::unix::io::RawFd;

use chrono::{DateTime, TimeZone, SecondsFormat, Utc};

use std::path::{PathBuf, Path};
use lazy_static::lazy_static;

use super::manifest::MANIFEST_BLOB_NAME;

macro_rules! BACKUP_ID_RE { () => (r"[A-Za-z0-9][A-Za-z0-9_-]+") }
macro_rules! BACKUP_TYPE_RE { () => (r"(?:host|vm|ct)") }
macro_rules! BACKUP_TIME_RE { () => (r"[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z") }

lazy_static!{
    static ref BACKUP_FILE_REGEX: Regex = Regex::new(
        r"^.*\.([fd]idx|blob)$").unwrap();

    static ref BACKUP_TYPE_REGEX: Regex = Regex::new(
        concat!(r"^(", BACKUP_TYPE_RE!(), r")$")).unwrap();

    static ref BACKUP_ID_REGEX: Regex = Regex::new(
        concat!(r"^", BACKUP_ID_RE!(), r"$")).unwrap();

    static ref BACKUP_DATE_REGEX: Regex = Regex::new(
        concat!(r"^", BACKUP_TIME_RE!() ,r"$")).unwrap();

    static ref GROUP_PATH_REGEX: Regex = Regex::new(
        concat!(r"^(", BACKUP_TYPE_RE!(), ")/(", BACKUP_ID_RE!(), r")$")).unwrap();

    static ref SNAPSHOT_PATH_REGEX: Regex = Regex::new(
        concat!(r"^(", BACKUP_TYPE_RE!(), ")/(", BACKUP_ID_RE!(), ")/(", BACKUP_TIME_RE!(), r")$")).unwrap();

}

/// BackupGroup is a directory containing a list of BackupDir
#[derive(Debug, Eq, PartialEq, Hash, Clone)]
pub struct BackupGroup {
    /// Type of backup
    backup_type: String,
    /// Unique (for this type) ID
    backup_id: String,
}

impl BackupGroup {

    pub fn new<T: Into<String>, U: Into<String>>(backup_type: T, backup_id: U) -> Self {
        Self { backup_type: backup_type.into(), backup_id: backup_id.into() }
    }

    pub fn backup_type(&self) -> &str {
        &self.backup_type
    }

    pub fn backup_id(&self) -> &str {
        &self.backup_id
    }

    pub fn group_path(&self) ->  PathBuf  {

        let mut relative_path = PathBuf::new();

        relative_path.push(&self.backup_type);

        relative_path.push(&self.backup_id);

        relative_path
    }

    pub fn list_backups(&self, base_path: &Path) -> Result<Vec<BackupInfo>, Error> {

        let mut list = vec![];

        let mut path = base_path.to_owned();
        path.push(self.group_path());

        tools::scandir(libc::AT_FDCWD, &path, &BACKUP_DATE_REGEX, |l2_fd, backup_time, file_type| {
            if file_type != nix::dir::Type::Directory { return Ok(()); }

            let dt = backup_time.parse::<DateTime<Utc>>()?;
            let backup_dir = BackupDir::new(self.backup_type.clone(), self.backup_id.clone(), dt.timestamp());
            let files = list_backup_files(l2_fd, backup_time)?;

            list.push(BackupInfo { backup_dir, files });

            Ok(())
        })?;
        Ok(list)
    }

    pub fn last_successful_backup(&self,  base_path: &Path) -> Result<Option<DateTime<Utc>>, Error> {

        let mut last = None;

        let mut path = base_path.to_owned();
        path.push(self.group_path());

        tools::scandir(libc::AT_FDCWD, &path, &BACKUP_DATE_REGEX, |l2_fd, backup_time, file_type| {
            if file_type != nix::dir::Type::Directory { return Ok(()); }

            let mut manifest_path = PathBuf::from(backup_time);
            manifest_path.push(MANIFEST_BLOB_NAME);

            use nix::fcntl::{openat, OFlag};
            match openat(l2_fd, &manifest_path, OFlag::O_RDONLY, nix::sys::stat::Mode::empty()) {
                Ok(rawfd) => {
                    /* manifest exists --> assume backup was successful */
                    /* close else this leaks! */
                    nix::unistd::close(rawfd)?;
                },
                Err(nix::Error::Sys(nix::errno::Errno::ENOENT)) => { return Ok(()); }
                Err(err) => {
                    bail!("last_successful_backup: unexpected error - {}", err);
                }
            }

            let dt = backup_time.parse::<DateTime<Utc>>()?;
            if let Some(last_dt) = last {
                if dt > last_dt { last = Some(dt); }
            } else {
                last = Some(dt);
            }

            Ok(())
        })?;

        Ok(last)
    }

    pub fn list_groups(base_path: &Path) -> Result<Vec<BackupGroup>, Error> {
        let mut list = Vec::new();

        tools::scandir(libc::AT_FDCWD, base_path, &BACKUP_TYPE_REGEX, |l0_fd, backup_type, file_type| {
            if file_type != nix::dir::Type::Directory { return Ok(()); }
            tools::scandir(l0_fd, backup_type, &BACKUP_ID_REGEX, |_l1_fd, backup_id, file_type| {
                if file_type != nix::dir::Type::Directory { return Ok(()); }
                list.push(BackupGroup::new(backup_type, backup_id));
                Ok(())
            })
        })?;
        Ok(list)
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
        let cap = GROUP_PATH_REGEX.captures(path)
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
    backup_time: DateTime<Utc>,
}

impl BackupDir {

    pub fn new<T, U>(backup_type: T, backup_id: U, timestamp: i64) -> Self
    where
        T: Into<String>,
        U: Into<String>,
    {
        // Note: makes sure that nanoseconds is 0
        Self {
            group: BackupGroup::new(backup_type.into(), backup_id.into()),
            backup_time: Utc.timestamp(timestamp, 0),
        }
    }
    pub fn new_with_group(group: BackupGroup, timestamp: i64) -> Self {
        Self { group, backup_time: Utc.timestamp(timestamp, 0) }
    }

    pub fn group(&self) -> &BackupGroup {
        &self.group
    }

    pub fn backup_time(&self) -> DateTime<Utc> {
        self.backup_time
    }

    pub fn relative_path(&self) ->  PathBuf  {

        let mut relative_path = self.group.group_path();

        relative_path.push(Self::backup_time_to_string(self.backup_time));

        relative_path
    }

    pub fn backup_time_to_string(backup_time: DateTime<Utc>) -> String {
        backup_time.to_rfc3339_opts(SecondsFormat::Secs, true)
    }
}

impl std::str::FromStr for BackupDir {
    type Err = Error;

    /// Parse a snapshot path
    ///
    /// This parses strings like `host/elsa/2020-06-15T05:18:33Z".
    fn from_str(path: &str) -> Result<Self, Self::Err> {
        let cap = SNAPSHOT_PATH_REGEX.captures(path)
            .ok_or_else(|| format_err!("unable to parse backup snapshot path '{}'", path))?;

        let group = BackupGroup::new(cap.get(1).unwrap().as_str(), cap.get(2).unwrap().as_str());
        let backup_time = cap.get(3).unwrap().as_str().parse::<DateTime<Utc>>()?;
        Ok(BackupDir::from((group, backup_time.timestamp())))
    }
}

impl std::fmt::Display for BackupDir {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let backup_type = self.group.backup_type();
        let id = self.group.backup_id();
        let time = Self::backup_time_to_string(self.backup_time);
        write!(f, "{}/{}/{}", backup_type, id, time)
    }
}

impl From<(BackupGroup, i64)> for BackupDir {
    fn from((group, timestamp): (BackupGroup, i64)) -> Self {
        Self { group, backup_time: Utc.timestamp(timestamp, 0) }
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
    pub fn last_backup(base_path: &Path, group: &BackupGroup) -> Result<Option<BackupInfo>, Error> {
        let backups = group.list_backups(base_path)?;
        Ok(backups.into_iter().max_by_key(|item| item.backup_dir.backup_time()))
    }

    pub fn sort_list(list: &mut Vec<BackupInfo>, ascendending: bool) {
        if ascendending { // oldest first
            list.sort_unstable_by(|a, b| a.backup_dir.backup_time.cmp(&b.backup_dir.backup_time));
        } else { // newest first
            list.sort_unstable_by(|a, b| b.backup_dir.backup_time.cmp(&a.backup_dir.backup_time));
        }
    }

    pub fn list_files(base_path: &Path, backup_dir: &BackupDir) -> Result<Vec<String>, Error> {
        let mut path = base_path.to_owned();
        path.push(backup_dir.relative_path());

        let files = list_backup_files(libc::AT_FDCWD, &path)?;

        Ok(files)
    }

    pub fn list_backups(base_path: &Path) -> Result<Vec<BackupInfo>, Error> {
        let mut list = Vec::new();

        tools::scandir(libc::AT_FDCWD, base_path, &BACKUP_TYPE_REGEX, |l0_fd, backup_type, file_type| {
            if file_type != nix::dir::Type::Directory { return Ok(()); }
            tools::scandir(l0_fd, backup_type, &BACKUP_ID_REGEX, |l1_fd, backup_id, file_type| {
                if file_type != nix::dir::Type::Directory { return Ok(()); }
                tools::scandir(l1_fd, backup_id, &BACKUP_DATE_REGEX, |l2_fd, backup_time, file_type| {
                    if file_type != nix::dir::Type::Directory { return Ok(()); }

                    let dt = backup_time.parse::<DateTime<Utc>>()?;
                    let backup_dir = BackupDir::new(backup_type, backup_id, dt.timestamp());

                    let files = list_backup_files(l2_fd, backup_time)?;

                    list.push(BackupInfo { backup_dir, files });

                    Ok(())
                })
            })
        })?;
        Ok(list)
    }

    pub fn is_finished(&self) -> bool {
        // backup is considered unfinished if there is no manifest
        self.files.iter().any(|name| name == super::MANIFEST_BLOB_NAME)
    }
}

fn list_backup_files<P: ?Sized + nix::NixPath>(dirfd: RawFd, path: &P) -> Result<Vec<String>, Error> {
    let mut files = vec![];

    tools::scandir(dirfd, path, &BACKUP_FILE_REGEX, |_, filename, file_type| {
        if file_type != nix::dir::Type::File { return Ok(()); }
        files.push(filename.to_owned());
        Ok(())
    })?;

    Ok(files)
}
