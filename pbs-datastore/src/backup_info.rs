use std::fmt;
use std::os::unix::io::RawFd;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{bail, format_err, Error};

use proxmox_sys::fs::{lock_dir_noblock, replace_file, CreateOptions};

use pbs_api_types::{
    Authid, BackupNamespace, BackupType, GroupFilter, BACKUP_DATE_REGEX, BACKUP_FILE_REGEX,
};
use pbs_config::{open_backup_lockfile, BackupLockGuard};

use crate::manifest::{
    BackupManifest, CLIENT_LOG_BLOB_NAME, MANIFEST_BLOB_NAME, MANIFEST_LOCK_NAME,
};
use crate::{DataBlob, DataStore};

/// BackupGroup is a directory containing a list of BackupDir
#[derive(Clone)]
pub struct BackupGroup {
    store: Arc<DataStore>,

    ns: BackupNamespace,
    group: pbs_api_types::BackupGroup,
}

impl fmt::Debug for BackupGroup {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("BackupGroup")
            .field("store", &self.store.name())
            .field("ns", &self.ns)
            .field("group", &self.group)
            .finish()
    }
}

impl BackupGroup {
    pub(crate) fn new(
        store: Arc<DataStore>,
        ns: BackupNamespace,
        group: pbs_api_types::BackupGroup,
    ) -> Self {
        Self { store, ns, group }
    }

    /// Access the underlying [`BackupGroup`](pbs_api_types::BackupGroup).
    #[inline]
    pub fn group(&self) -> &pbs_api_types::BackupGroup {
        &self.group
    }

    #[inline]
    pub fn backup_ns(&self) -> &BackupNamespace {
        &self.ns
    }

    #[inline]
    pub fn backup_type(&self) -> BackupType {
        self.group.ty
    }

    #[inline]
    pub fn backup_id(&self) -> &str {
        &self.group.id
    }

    pub fn full_group_path(&self) -> PathBuf {
        self.store.group_path(&self.ns, &self.group)
    }

    pub fn relative_group_path(&self) -> PathBuf {
        let mut path = self.ns.path();
        path.push(self.group.ty.as_str());
        path.push(&self.group.id);
        path
    }

    /// Simple check whether a group exists. This does not check whether there are any snapshots,
    /// but rather it simply checks whether the directory exists.
    pub fn exists(&self) -> bool {
        self.full_group_path().exists()
    }

    pub fn list_backups(&self) -> Result<Vec<BackupInfo>, Error> {
        let mut list = vec![];

        let path = self.full_group_path();

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

                let protected = backup_dir.is_protected();

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
    pub fn last_backup(&self, only_finished: bool) -> Result<Option<BackupInfo>, Error> {
        let backups = self.list_backups()?;
        Ok(backups
            .into_iter()
            .filter(|item| !only_finished || item.is_finished())
            .max_by_key(|item| item.backup_dir.backup_time()))
    }

    pub fn last_successful_backup(&self) -> Result<Option<i64>, Error> {
        let mut last = None;

        let path = self.full_group_path();

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
                    Err(nix::errno::Errno::ENOENT) => {
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

    pub fn iter_snapshots(&self) -> Result<crate::ListSnapshots, Error> {
        crate::ListSnapshots::new(self.clone())
    }

    /// Destroy the group inclusive all its backup snapshots (BackupDir's)
    ///
    /// Returns true if all snapshots were removed, and false if some were protected
    pub fn destroy(&self) -> Result<bool, Error> {
        let path = self.full_group_path();
        let _guard =
            proxmox_sys::fs::lock_dir_noblock(&path, "backup group", "possible running backup")?;

        log::info!("removing backup group {:?}", path);
        let mut removed_all_snaps = true;
        for snap in self.iter_snapshots()? {
            let snap = snap?;
            if snap.is_protected() {
                removed_all_snaps = false;
                continue;
            }
            snap.destroy(false)?;
        }

        if removed_all_snaps {
            std::fs::remove_dir_all(&path).map_err(|err| {
                format_err!("removing group directory {:?} failed - {}", path, err)
            })?;
        }

        Ok(removed_all_snaps)
    }

    /// Returns the backup owner.
    ///
    /// The backup owner is the entity who first created the backup group.
    pub fn get_owner(&self) -> Result<Authid, Error> {
        self.store.get_owner(&self.ns, self.as_ref())
    }

    /// Set the backup owner.
    pub fn set_owner(&self, auth_id: &Authid, force: bool) -> Result<(), Error> {
        self.store
            .set_owner(&self.ns, self.as_ref(), auth_id, force)
    }
}

impl AsRef<pbs_api_types::BackupNamespace> for BackupGroup {
    #[inline]
    fn as_ref(&self) -> &pbs_api_types::BackupNamespace {
        &self.ns
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

impl From<BackupDir> for BackupGroup {
    fn from(dir: BackupDir) -> BackupGroup {
        BackupGroup {
            store: dir.store,
            ns: dir.ns,
            group: dir.dir.group,
        }
    }
}

impl From<&BackupDir> for BackupGroup {
    fn from(dir: &BackupDir) -> BackupGroup {
        BackupGroup {
            store: Arc::clone(&dir.store),
            ns: dir.ns.clone(),
            group: dir.dir.group.clone(),
        }
    }
}

/// Uniquely identify a Backup (relative to data store)
///
/// We also call this a backup snaphost.
#[derive(Clone)]
pub struct BackupDir {
    store: Arc<DataStore>,
    ns: BackupNamespace,
    dir: pbs_api_types::BackupDir,
    // backup_time as rfc3339
    backup_time_string: String,
}

impl fmt::Debug for BackupDir {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("BackupDir")
            .field("store", &self.store.name())
            .field("ns", &self.ns)
            .field("dir", &self.dir)
            .field("backup_time_string", &self.backup_time_string)
            .finish()
    }
}

impl BackupDir {
    /// Temporarily used for tests.
    #[doc(hidden)]
    pub fn new_test(dir: pbs_api_types::BackupDir) -> Self {
        Self {
            store: unsafe { DataStore::new_test() },
            backup_time_string: Self::backup_time_to_string(dir.time).unwrap(),
            ns: BackupNamespace::root(),
            dir,
        }
    }

    pub(crate) fn with_group(group: BackupGroup, backup_time: i64) -> Result<Self, Error> {
        let backup_time_string = Self::backup_time_to_string(backup_time)?;
        Ok(Self {
            store: group.store,
            ns: group.ns,
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
            store: group.store,
            ns: group.ns,
            dir: (group.group, backup_time).into(),
            backup_time_string,
        })
    }

    #[inline]
    pub fn backup_ns(&self) -> &BackupNamespace {
        &self.ns
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

    pub fn dir(&self) -> &pbs_api_types::BackupDir {
        &self.dir
    }

    pub fn group(&self) -> &pbs_api_types::BackupGroup {
        &self.dir.group
    }

    pub fn relative_path(&self) -> PathBuf {
        let mut path = self.ns.path();
        path.push(self.dir.group.ty.as_str());
        path.push(&self.dir.group.id);
        path.push(&self.backup_time_string);
        path
    }

    /// Returns the absolute path for backup_dir, using the cached formatted time string.
    pub fn full_path(&self) -> PathBuf {
        self.store.snapshot_path(&self.ns, &self.dir)
    }

    pub fn protected_file(&self) -> PathBuf {
        let mut path = self.full_path();
        path.push(".protected");
        path
    }

    pub fn is_protected(&self) -> bool {
        let path = self.protected_file();
        path.exists()
    }

    pub fn backup_time_to_string(backup_time: i64) -> Result<String, Error> {
        // fixme: can this fail? (avoid unwrap)
        proxmox_time::epoch_to_rfc3339_utc(backup_time)
    }

    /// load a `DataBlob` from this snapshot's backup dir.
    pub fn load_blob(&self, filename: &str) -> Result<DataBlob, Error> {
        let mut path = self.full_path();
        path.push(filename);

        proxmox_lang::try_block!({
            let mut file = std::fs::File::open(&path)?;
            DataBlob::load_from_reader(&mut file)
        })
        .map_err(|err| format_err!("unable to load blob '{:?}' - {}", path, err))
    }

    /// Returns the filename to lock a manifest
    ///
    /// Also creates the basedir. The lockfile is located in
    /// '/run/proxmox-backup/locks/{datastore}/[ns/{ns}/]+{type}/{id}/{timestamp}.index.json.lck'
    fn manifest_lock_path(&self) -> Result<PathBuf, Error> {
        let mut path = PathBuf::from(&format!("/run/proxmox-backup/locks/{}", self.store.name()));
        path.push(self.relative_path());

        std::fs::create_dir_all(&path)?;
        let ts = self.backup_time_string();
        path.push(&format!("{ts}{MANIFEST_LOCK_NAME}"));

        Ok(path)
    }

    /// Locks the manifest of a snapshot, for example, to update or delete it.
    pub(crate) fn lock_manifest(&self) -> Result<BackupLockGuard, Error> {
        let path = self.manifest_lock_path()?;

        // actions locking the manifest should be relatively short, only wait a few seconds
        open_backup_lockfile(&path, Some(std::time::Duration::from_secs(5)), true)
            .map_err(|err| format_err!("unable to acquire manifest lock {:?} - {}", &path, err))
    }

    /// Destroy the whole snapshot, bails if it's protected
    ///
    /// Setting `force` to true skips locking and thus ignores if the backup is currently in use.
    pub fn destroy(&self, force: bool) -> Result<(), Error> {
        let full_path = self.full_path();

        let (_guard, _manifest_guard);
        if !force {
            _guard = lock_dir_noblock(&full_path, "snapshot", "possibly running or in use")?;
            _manifest_guard = self.lock_manifest()?;
        }

        if self.is_protected() {
            bail!("cannot remove protected snapshot"); // use special error type?
        }

        log::info!("removing backup snapshot {:?}", full_path);
        std::fs::remove_dir_all(&full_path).map_err(|err| {
            format_err!("removing backup snapshot {:?} failed - {}", full_path, err,)
        })?;

        // the manifest doesn't exist anymore, no need to keep the lock (already done by guard?)
        if let Ok(path) = self.manifest_lock_path() {
            let _ = std::fs::remove_file(path); // ignore errors
        }

        Ok(())
    }

    /// Get the datastore.
    pub fn datastore(&self) -> &Arc<DataStore> {
        &self.store
    }

    /// Returns the backup owner.
    ///
    /// The backup owner is the entity who first created the backup group.
    pub fn get_owner(&self) -> Result<Authid, Error> {
        self.store.get_owner(&self.ns, self.as_ref())
    }

    /// Lock the snapshot and open a reader.
    pub fn locked_reader(&self) -> Result<crate::SnapshotReader, Error> {
        crate::SnapshotReader::new_do(self.clone())
    }

    /// Load the manifest without a lock. Must not be written back.
    pub fn load_manifest(&self) -> Result<(BackupManifest, u64), Error> {
        let blob = self.load_blob(MANIFEST_BLOB_NAME)?;
        let raw_size = blob.raw_size();
        let manifest = BackupManifest::try_from(blob)?;
        Ok((manifest, raw_size))
    }

    /// Update the manifest of the specified snapshot. Never write a manifest directly,
    /// only use this method - anything else may break locking guarantees.
    pub fn update_manifest(
        &self,
        update_fn: impl FnOnce(&mut BackupManifest),
    ) -> Result<(), Error> {
        let _guard = self.lock_manifest()?;
        let (mut manifest, _) = self.load_manifest()?;

        update_fn(&mut manifest);

        let manifest = serde_json::to_value(manifest)?;
        let manifest = serde_json::to_string_pretty(&manifest)?;
        let blob = DataBlob::encode(manifest.as_bytes(), None, true)?;
        let raw_data = blob.raw_data();

        let mut path = self.full_path();
        path.push(MANIFEST_BLOB_NAME);

        // atomic replace invalidates flock - no other writes past this point!
        replace_file(&path, raw_data, CreateOptions::new(), false)?;
        Ok(())
    }

    /// Cleans up the backup directory by removing any file not mentioned in the manifest.
    pub fn cleanup_unreferenced_files(&self, manifest: &BackupManifest) -> Result<(), Error> {
        let full_path = self.full_path();

        let mut wanted_files = std::collections::HashSet::new();
        wanted_files.insert(MANIFEST_BLOB_NAME.to_string());
        wanted_files.insert(CLIENT_LOG_BLOB_NAME.to_string());
        manifest.files().iter().for_each(|item| {
            wanted_files.insert(item.filename.clone());
        });

        for item in proxmox_sys::fs::read_subdir(libc::AT_FDCWD, &full_path)?.flatten() {
            if let Some(file_type) = item.file_type() {
                if file_type != nix::dir::Type::File {
                    continue;
                }
            }
            let file_name = item.file_name().to_bytes();
            if file_name == b"." || file_name == b".." {
                continue;
            };
            if let Ok(name) = std::str::from_utf8(file_name) {
                if wanted_files.contains(name) {
                    continue;
                }
            }
            println!("remove unused file {:?}", item.file_name());
            let dirfd = item.parent_fd();
            let _res = unsafe { libc::unlinkat(dirfd, item.file_name().as_ptr(), 0) };
        }

        Ok(())
    }
}

impl AsRef<pbs_api_types::BackupNamespace> for BackupDir {
    fn as_ref(&self) -> &pbs_api_types::BackupNamespace {
        &self.ns
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
        dir.dir.group
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

/// Detailed Backup Information, lists files inside a BackupDir
#[derive(Clone, Debug)]
pub struct BackupInfo {
    /// the backup directory
    pub backup_dir: BackupDir,
    /// List of data files
    pub files: Vec<String>,
    /// Protection Status
    pub protected: bool,
}

impl BackupInfo {
    pub fn new(backup_dir: BackupDir) -> Result<BackupInfo, Error> {
        let path = backup_dir.full_path();

        let files = list_backup_files(libc::AT_FDCWD, &path)?;
        let protected = backup_dir.is_protected();

        Ok(BackupInfo {
            backup_dir,
            files,
            protected,
        })
    }

    pub fn sort_list(list: &mut [BackupInfo], ascendending: bool) {
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
