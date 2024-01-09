use std::collections::{HashMap, HashSet};
use std::io::{self, Write};
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{bail, format_err, Error};
use lazy_static::lazy_static;
use nix::unistd::{unlinkat, UnlinkatFlags};

use proxmox_human_byte::HumanByte;
use proxmox_schema::ApiType;

use proxmox_sys::error::SysError;
use proxmox_sys::fs::{file_read_optional_string, replace_file, CreateOptions};
use proxmox_sys::fs::{lock_dir_noblock, DirLockGuard};
use proxmox_sys::process_locker::ProcessLockSharedGuard;
use proxmox_sys::WorkerTaskContext;
use proxmox_sys::{task_log, task_warn};

use pbs_api_types::{
    Authid, BackupNamespace, BackupType, ChunkOrder, DataStoreConfig, DatastoreFSyncLevel,
    DatastoreTuning, GarbageCollectionStatus, Operation, UPID,
};

use crate::backup_info::{BackupDir, BackupGroup};
use crate::chunk_store::ChunkStore;
use crate::dynamic_index::{DynamicIndexReader, DynamicIndexWriter};
use crate::fixed_index::{FixedIndexReader, FixedIndexWriter};
use crate::hierarchy::{ListGroups, ListGroupsType, ListNamespaces, ListNamespacesRecursive};
use crate::index::IndexFile;
use crate::manifest::{archive_type, ArchiveType};
use crate::task_tracking::{self, update_active_operations};
use crate::DataBlob;

lazy_static! {
    static ref DATASTORE_MAP: Mutex<HashMap<String, Arc<DataStoreImpl>>> =
        Mutex::new(HashMap::new());
}

/// checks if auth_id is owner, or, if owner is a token, if
/// auth_id is the user of the token
pub fn check_backup_owner(owner: &Authid, auth_id: &Authid) -> Result<(), Error> {
    let correct_owner =
        owner == auth_id || (owner.is_token() && &Authid::from(owner.user().clone()) == auth_id);
    if !correct_owner {
        bail!("backup owner check failed ({} != {})", auth_id, owner);
    }
    Ok(())
}

/// Datastore Management
///
/// A Datastore can store severals backups, and provides the
/// management interface for backup.
pub struct DataStoreImpl {
    chunk_store: Arc<ChunkStore>,
    gc_mutex: Mutex<()>,
    last_gc_status: Mutex<GarbageCollectionStatus>,
    verify_new: bool,
    chunk_order: ChunkOrder,
    last_digest: Option<[u8; 32]>,
    sync_level: DatastoreFSyncLevel,
}

impl DataStoreImpl {
    // This one just panics on everything
    #[doc(hidden)]
    pub(crate) unsafe fn new_test() -> Arc<Self> {
        Arc::new(Self {
            chunk_store: Arc::new(unsafe { ChunkStore::panic_store() }),
            gc_mutex: Mutex::new(()),
            last_gc_status: Mutex::new(GarbageCollectionStatus::default()),
            verify_new: false,
            chunk_order: Default::default(),
            last_digest: None,
            sync_level: Default::default(),
        })
    }
}

pub struct DataStore {
    inner: Arc<DataStoreImpl>,
    operation: Option<Operation>,
}

impl Clone for DataStore {
    fn clone(&self) -> Self {
        let mut new_operation = self.operation;
        if let Some(operation) = self.operation {
            if let Err(e) = update_active_operations(self.name(), operation, 1) {
                log::error!("could not update active operations - {}", e);
                new_operation = None;
            }
        }

        DataStore {
            inner: self.inner.clone(),
            operation: new_operation,
        }
    }
}

impl Drop for DataStore {
    fn drop(&mut self) {
        if let Some(operation) = self.operation {
            if let Err(e) = update_active_operations(self.name(), operation, -1) {
                log::error!("could not update active operations - {}", e);
            }
        }
    }
}

impl DataStore {
    // This one just panics on everything
    #[doc(hidden)]
    pub(crate) unsafe fn new_test() -> Arc<Self> {
        Arc::new(Self {
            inner: unsafe { DataStoreImpl::new_test() },
            operation: None,
        })
    }

    pub fn lookup_datastore(
        name: &str,
        operation: Option<Operation>,
    ) -> Result<Arc<DataStore>, Error> {
        // Avoid TOCTOU between checking maintenance mode and updating active operation counter, as
        // we use it to decide whether it is okay to delete the datastore.
        let config_lock = pbs_config::datastore::lock_config()?;

        // we could use the ConfigVersionCache's generation for staleness detection, but  we load
        // the config anyway -> just use digest, additional benefit: manual changes get detected
        let (config, digest) = pbs_config::datastore::config()?;
        let config: DataStoreConfig = config.lookup("datastore", name)?;

        if let Some(maintenance_mode) = config.get_maintenance_mode() {
            if let Err(error) = maintenance_mode.check(operation) {
                bail!("datastore '{name}' is in {error}");
            }
        }

        if let Some(operation) = operation {
            update_active_operations(name, operation, 1)?;
        }

        // Our operation is registered, unlock the config.
        drop(config_lock);

        let mut datastore_cache = DATASTORE_MAP.lock().unwrap();
        let entry = datastore_cache.get(name);

        // reuse chunk store so that we keep using the same process locker instance!
        let chunk_store = if let Some(datastore) = &entry {
            let last_digest = datastore.last_digest.as_ref();
            if let Some(true) = last_digest.map(|last_digest| last_digest == &digest) {
                return Ok(Arc::new(Self {
                    inner: Arc::clone(datastore),
                    operation,
                }));
            }
            Arc::clone(&datastore.chunk_store)
        } else {
            let tuning: DatastoreTuning = serde_json::from_value(
                DatastoreTuning::API_SCHEMA
                    .parse_property_string(config.tuning.as_deref().unwrap_or(""))?,
            )?;
            Arc::new(ChunkStore::open(
                name,
                &config.path,
                tuning.sync_level.unwrap_or_default(),
            )?)
        };

        let datastore = DataStore::with_store_and_config(chunk_store, config, Some(digest))?;

        let datastore = Arc::new(datastore);
        datastore_cache.insert(name.to_string(), datastore.clone());

        Ok(Arc::new(Self {
            inner: datastore,
            operation,
        }))
    }

    /// removes all datastores that are not configured anymore
    pub fn remove_unused_datastores() -> Result<(), Error> {
        let (config, _digest) = pbs_config::datastore::config()?;

        let mut map = DATASTORE_MAP.lock().unwrap();
        // removes all elements that are not in the config
        map.retain(|key, _| config.sections.contains_key(key));
        Ok(())
    }

    /// Open a raw database given a name and a path.
    ///
    /// # Safety
    /// See the safety section in `open_from_config`
    pub unsafe fn open_path(
        name: &str,
        path: impl AsRef<Path>,
        operation: Option<Operation>,
    ) -> Result<Arc<Self>, Error> {
        let path = path
            .as_ref()
            .to_str()
            .ok_or_else(|| format_err!("non-utf8 paths not supported"))?
            .to_owned();
        unsafe { Self::open_from_config(DataStoreConfig::new(name.to_owned(), path), operation) }
    }

    /// Open a datastore given a raw configuration.
    ///
    /// # Safety
    /// There's no memory safety implication, but as this is opening a new ChunkStore it will
    /// create a new process locker instance, potentially on the same path as existing safely
    /// created ones. This is dangerous as dropping the reference of this and thus the underlying
    /// chunkstore's process locker will close all locks from our process on the config.path,
    /// breaking guarantees we need to uphold for safe long backup + GC interaction on newer/older
    /// process instances (from package update).
    unsafe fn open_from_config(
        config: DataStoreConfig,
        operation: Option<Operation>,
    ) -> Result<Arc<Self>, Error> {
        let name = config.name.clone();

        let tuning: DatastoreTuning = serde_json::from_value(
            DatastoreTuning::API_SCHEMA
                .parse_property_string(config.tuning.as_deref().unwrap_or(""))?,
        )?;
        let chunk_store =
            ChunkStore::open(&name, &config.path, tuning.sync_level.unwrap_or_default())?;
        let inner = Arc::new(Self::with_store_and_config(
            Arc::new(chunk_store),
            config,
            None,
        )?);

        if let Some(operation) = operation {
            update_active_operations(&name, operation, 1)?;
        }

        Ok(Arc::new(Self { inner, operation }))
    }

    fn with_store_and_config(
        chunk_store: Arc<ChunkStore>,
        config: DataStoreConfig,
        last_digest: Option<[u8; 32]>,
    ) -> Result<DataStoreImpl, Error> {
        let mut gc_status_path = chunk_store.base_path();
        gc_status_path.push(".gc-status");

        let gc_status = if let Some(state) = file_read_optional_string(gc_status_path)? {
            match serde_json::from_str(&state) {
                Ok(state) => state,
                Err(err) => {
                    log::error!("error reading gc-status: {}", err);
                    GarbageCollectionStatus::default()
                }
            }
        } else {
            GarbageCollectionStatus::default()
        };

        let tuning: DatastoreTuning = serde_json::from_value(
            DatastoreTuning::API_SCHEMA
                .parse_property_string(config.tuning.as_deref().unwrap_or(""))?,
        )?;

        Ok(DataStoreImpl {
            chunk_store,
            gc_mutex: Mutex::new(()),
            last_gc_status: Mutex::new(gc_status),
            verify_new: config.verify_new.unwrap_or(false),
            chunk_order: tuning.chunk_order.unwrap_or_default(),
            last_digest,
            sync_level: tuning.sync_level.unwrap_or_default(),
        })
    }

    pub fn get_chunk_iterator(
        &self,
    ) -> Result<
        impl Iterator<Item = (Result<proxmox_sys::fs::ReadDirEntry, Error>, usize, bool)>,
        Error,
    > {
        self.inner.chunk_store.get_chunk_iterator()
    }

    pub fn create_fixed_writer<P: AsRef<Path>>(
        &self,
        filename: P,
        size: usize,
        chunk_size: usize,
    ) -> Result<FixedIndexWriter, Error> {
        let index = FixedIndexWriter::create(
            self.inner.chunk_store.clone(),
            filename.as_ref(),
            size,
            chunk_size,
        )?;

        Ok(index)
    }

    pub fn open_fixed_reader<P: AsRef<Path>>(
        &self,
        filename: P,
    ) -> Result<FixedIndexReader, Error> {
        let full_path = self.inner.chunk_store.relative_path(filename.as_ref());

        let index = FixedIndexReader::open(&full_path)?;

        Ok(index)
    }

    pub fn create_dynamic_writer<P: AsRef<Path>>(
        &self,
        filename: P,
    ) -> Result<DynamicIndexWriter, Error> {
        let index = DynamicIndexWriter::create(self.inner.chunk_store.clone(), filename.as_ref())?;

        Ok(index)
    }

    pub fn open_dynamic_reader<P: AsRef<Path>>(
        &self,
        filename: P,
    ) -> Result<DynamicIndexReader, Error> {
        let full_path = self.inner.chunk_store.relative_path(filename.as_ref());

        let index = DynamicIndexReader::open(&full_path)?;

        Ok(index)
    }

    pub fn open_index<P>(&self, filename: P) -> Result<Box<dyn IndexFile + Send>, Error>
    where
        P: AsRef<Path>,
    {
        let filename = filename.as_ref();
        let out: Box<dyn IndexFile + Send> = match archive_type(filename)? {
            ArchiveType::DynamicIndex => Box::new(self.open_dynamic_reader(filename)?),
            ArchiveType::FixedIndex => Box::new(self.open_fixed_reader(filename)?),
            _ => bail!("cannot open index file of unknown type: {:?}", filename),
        };
        Ok(out)
    }

    /// Fast index verification - only check if chunks exists
    pub fn fast_index_verification(
        &self,
        index: &dyn IndexFile,
        checked: &mut HashSet<[u8; 32]>,
    ) -> Result<(), Error> {
        for pos in 0..index.index_count() {
            let info = index.chunk_info(pos).unwrap();
            if checked.contains(&info.digest) {
                continue;
            }

            self.stat_chunk(&info.digest).map_err(|err| {
                format_err!(
                    "fast_index_verification error, stat_chunk {} failed - {}",
                    hex::encode(info.digest),
                    err,
                )
            })?;

            checked.insert(info.digest);
        }

        Ok(())
    }

    pub fn name(&self) -> &str {
        self.inner.chunk_store.name()
    }

    pub fn base_path(&self) -> PathBuf {
        self.inner.chunk_store.base_path()
    }

    /// Returns the absolute path for a backup namespace on this datastore
    pub fn namespace_path(&self, ns: &BackupNamespace) -> PathBuf {
        let mut path = self.base_path();
        path.reserve(ns.path_len());
        for part in ns.components() {
            path.push("ns");
            path.push(part);
        }
        path
    }

    /// Returns the absolute path for a backup_type
    pub fn type_path(&self, ns: &BackupNamespace, backup_type: BackupType) -> PathBuf {
        let mut full_path = self.namespace_path(ns);
        full_path.push(backup_type.to_string());
        full_path
    }

    /// Returns the absolute path for a backup_group
    pub fn group_path(
        &self,
        ns: &BackupNamespace,
        backup_group: &pbs_api_types::BackupGroup,
    ) -> PathBuf {
        let mut full_path = self.namespace_path(ns);
        full_path.push(backup_group.to_string());
        full_path
    }

    /// Returns the absolute path for backup_dir
    pub fn snapshot_path(
        &self,
        ns: &BackupNamespace,
        backup_dir: &pbs_api_types::BackupDir,
    ) -> PathBuf {
        let mut full_path = self.namespace_path(ns);
        full_path.push(backup_dir.to_string());
        full_path
    }

    /// Create a backup namespace.
    pub fn create_namespace(
        self: &Arc<Self>,
        parent: &BackupNamespace,
        name: String,
    ) -> Result<BackupNamespace, Error> {
        if !self.namespace_exists(parent) {
            bail!("cannot create new namespace, parent {parent} doesn't already exists");
        }

        // construct ns before mkdir to enforce max-depth and name validity
        let ns = BackupNamespace::from_parent_ns(parent, name)?;

        let mut ns_full_path = self.base_path();
        ns_full_path.push(ns.path());

        std::fs::create_dir_all(ns_full_path)?;

        Ok(ns)
    }

    /// Returns if the given namespace exists on the datastore
    pub fn namespace_exists(&self, ns: &BackupNamespace) -> bool {
        let mut path = self.base_path();
        path.push(ns.path());
        path.exists()
    }

    /// Remove all backup groups of a single namespace level but not the namespace itself.
    ///
    /// Does *not* descends into child-namespaces and doesn't remoes the namespace itself either.
    ///
    /// Returns true if all the groups were removed, and false if some were protected.
    pub fn remove_namespace_groups(self: &Arc<Self>, ns: &BackupNamespace) -> Result<bool, Error> {
        // FIXME: locking? The single groups/snapshots are already protected, so may not be
        // necessary (depends on what we all allow to do with namespaces)
        log::info!("removing all groups in namespace {}:/{ns}", self.name());

        let mut removed_all_groups = true;

        for group in self.iter_backup_groups(ns.to_owned())? {
            let removed_group = group?.destroy()?;
            removed_all_groups = removed_all_groups && removed_group;
        }

        let base_file = std::fs::File::open(self.base_path())?;
        let base_fd = base_file.as_raw_fd();
        for ty in BackupType::iter() {
            let mut ty_dir = ns.path();
            ty_dir.push(ty.to_string());
            // best effort only, but we probably should log the error
            if let Err(err) = unlinkat(Some(base_fd), &ty_dir, UnlinkatFlags::RemoveDir) {
                if err != nix::errno::Errno::ENOENT {
                    log::error!("failed to remove backup type {ty} in {ns} - {err}");
                }
            }
        }

        Ok(removed_all_groups)
    }

    /// Remove a complete backup namespace optionally including all it's, and child namespaces',
    /// groups. If  `removed_groups` is false this only prunes empty namespaces.
    ///
    /// Returns true if everything requested, and false if some groups were protected or if some
    /// namespaces weren't empty even though all groups were deleted (race with new backup)
    pub fn remove_namespace_recursive(
        self: &Arc<Self>,
        ns: &BackupNamespace,
        delete_groups: bool,
    ) -> Result<bool, Error> {
        let store = self.name();
        let mut removed_all_requested = true;
        if delete_groups {
            log::info!("removing whole namespace recursively below {store}:/{ns}",);
            for ns in self.recursive_iter_backup_ns(ns.to_owned())? {
                let removed_ns_groups = self.remove_namespace_groups(&ns?)?;
                removed_all_requested = removed_all_requested && removed_ns_groups;
            }
        } else {
            log::info!("pruning empty namespace recursively below {store}:/{ns}");
        }

        // now try to delete the actual namespaces, bottom up so that we can use safe rmdir that
        // will choke if a new backup/group appeared in the meantime (but not on an new empty NS)
        let mut children = self
            .recursive_iter_backup_ns(ns.to_owned())?
            .collect::<Result<Vec<BackupNamespace>, Error>>()?;

        children.sort_by_key(|b| std::cmp::Reverse(b.depth()));

        let base_file = std::fs::File::open(self.base_path())?;
        let base_fd = base_file.as_raw_fd();

        for ns in children.iter() {
            let mut ns_dir = ns.path();
            ns_dir.push("ns");
            let _ = unlinkat(Some(base_fd), &ns_dir, UnlinkatFlags::RemoveDir);

            if !ns.is_root() {
                match unlinkat(Some(base_fd), &ns.path(), UnlinkatFlags::RemoveDir) {
                    Ok(()) => log::debug!("removed namespace {ns}"),
                    Err(nix::errno::Errno::ENOENT) => {
                        log::debug!("namespace {ns} already removed")
                    }
                    Err(nix::errno::Errno::ENOTEMPTY) if !delete_groups => {
                        removed_all_requested = false;
                        log::debug!("skip removal of non-empty namespace {ns}")
                    }
                    Err(err) => {
                        removed_all_requested = false;
                        log::warn!("failed to remove namespace {ns} - {err}")
                    }
                }
            }
        }

        Ok(removed_all_requested)
    }

    /// Remove a complete backup group including all snapshots.
    ///
    /// Returns true if all snapshots were removed, and false if some were protected
    pub fn remove_backup_group(
        self: &Arc<Self>,
        ns: &BackupNamespace,
        backup_group: &pbs_api_types::BackupGroup,
    ) -> Result<bool, Error> {
        let backup_group = self.backup_group(ns.clone(), backup_group.clone());

        backup_group.destroy()
    }

    /// Remove a backup directory including all content
    pub fn remove_backup_dir(
        self: &Arc<Self>,
        ns: &BackupNamespace,
        backup_dir: &pbs_api_types::BackupDir,
        force: bool,
    ) -> Result<(), Error> {
        let backup_dir = self.backup_dir(ns.clone(), backup_dir.clone())?;

        backup_dir.destroy(force)
    }

    /// Returns the time of the last successful backup
    ///
    /// Or None if there is no backup in the group (or the group dir does not exist).
    pub fn last_successful_backup(
        self: &Arc<Self>,
        ns: &BackupNamespace,
        backup_group: &pbs_api_types::BackupGroup,
    ) -> Result<Option<i64>, Error> {
        let backup_group = self.backup_group(ns.clone(), backup_group.clone());

        let group_path = backup_group.full_group_path();

        if group_path.exists() {
            backup_group.last_successful_backup()
        } else {
            Ok(None)
        }
    }

    /// Return the path of the 'owner' file.
    fn owner_path(&self, ns: &BackupNamespace, group: &pbs_api_types::BackupGroup) -> PathBuf {
        self.group_path(ns, group).join("owner")
    }

    /// Returns the backup owner.
    ///
    /// The backup owner is the entity who first created the backup group.
    pub fn get_owner(
        &self,
        ns: &BackupNamespace,
        backup_group: &pbs_api_types::BackupGroup,
    ) -> Result<Authid, Error> {
        let full_path = self.owner_path(ns, backup_group);
        let owner = proxmox_sys::fs::file_read_firstline(full_path)?;
        owner
            .trim_end() // remove trailing newline
            .parse()
            .map_err(|err| {
                format_err!("parsing owner for {backup_group} failed: {err}")
            })
    }

    pub fn owns_backup(
        &self,
        ns: &BackupNamespace,
        backup_group: &pbs_api_types::BackupGroup,
        auth_id: &Authid,
    ) -> Result<bool, Error> {
        let owner = self.get_owner(ns, backup_group)?;

        Ok(check_backup_owner(&owner, auth_id).is_ok())
    }

    /// Set the backup owner.
    pub fn set_owner(
        &self,
        ns: &BackupNamespace,
        backup_group: &pbs_api_types::BackupGroup,
        auth_id: &Authid,
        force: bool,
    ) -> Result<(), Error> {
        let path = self.owner_path(ns, backup_group);

        let mut open_options = std::fs::OpenOptions::new();
        open_options.write(true);
        open_options.truncate(true);

        if force {
            open_options.create(true);
        } else {
            open_options.create_new(true);
        }

        let mut file = open_options
            .open(&path)
            .map_err(|err| format_err!("unable to create owner file {:?} - {}", path, err))?;

        writeln!(file, "{}", auth_id)
            .map_err(|err| format_err!("unable to write owner file  {:?} - {}", path, err))?;

        Ok(())
    }

    /// Create (if it does not already exists) and lock a backup group
    ///
    /// And set the owner to 'userid'. If the group already exists, it returns the
    /// current owner (instead of setting the owner).
    ///
    /// This also acquires an exclusive lock on the directory and returns the lock guard.
    pub fn create_locked_backup_group(
        &self,
        ns: &BackupNamespace,
        backup_group: &pbs_api_types::BackupGroup,
        auth_id: &Authid,
    ) -> Result<(Authid, DirLockGuard), Error> {
        // create intermediate path first:
        let mut full_path = self.base_path();
        for ns in ns.components() {
            full_path.push("ns");
            full_path.push(ns);
        }
        full_path.push(backup_group.ty.as_str());
        std::fs::create_dir_all(&full_path)?;

        full_path.push(&backup_group.id);

        // create the last component now
        match std::fs::create_dir(&full_path) {
            Ok(_) => {
                let guard = lock_dir_noblock(
                    &full_path,
                    "backup group",
                    "another backup is already running",
                )?;
                self.set_owner(ns, backup_group, auth_id, false)?;
                let owner = self.get_owner(ns, backup_group)?; // just to be sure
                Ok((owner, guard))
            }
            Err(ref err) if err.kind() == io::ErrorKind::AlreadyExists => {
                let guard = lock_dir_noblock(
                    &full_path,
                    "backup group",
                    "another backup is already running",
                )?;
                let owner = self.get_owner(ns, backup_group)?; // just to be sure
                Ok((owner, guard))
            }
            Err(err) => bail!("unable to create backup group {:?} - {}", full_path, err),
        }
    }

    /// Creates a new backup snapshot inside a BackupGroup
    ///
    /// The BackupGroup directory needs to exist.
    pub fn create_locked_backup_dir(
        &self,
        ns: &BackupNamespace,
        backup_dir: &pbs_api_types::BackupDir,
    ) -> Result<(PathBuf, bool, DirLockGuard), Error> {
        let full_path = self.snapshot_path(ns, backup_dir);
        let relative_path = full_path.strip_prefix(self.base_path()).map_err(|err| {
            format_err!(
                "failed to produce correct path for backup {backup_dir} in namespace {ns}: {err}"
            )
        })?;

        let lock = || {
            lock_dir_noblock(
                &full_path,
                "snapshot",
                "internal error - tried creating snapshot that's already in use",
            )
        };

        match std::fs::create_dir(&full_path) {
            Ok(_) => Ok((relative_path.to_owned(), true, lock()?)),
            Err(ref e) if e.kind() == io::ErrorKind::AlreadyExists => {
                Ok((relative_path.to_owned(), false, lock()?))
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Get a streaming iter over single-level backup namespaces of a datatstore
    ///
    /// The iterated item is still a Result that can contain errors from rather unexptected FS or
    /// parsing errors.
    pub fn iter_backup_ns(
        self: &Arc<DataStore>,
        ns: BackupNamespace,
    ) -> Result<ListNamespaces, Error> {
        ListNamespaces::new(Arc::clone(self), ns)
    }

    /// Get a streaming iter over single-level backup namespaces of a datatstore, filtered by Ok
    ///
    /// The iterated item's result is already unwrapped, if it contained an error it will be
    /// logged. Can be useful in iterator chain commands
    pub fn iter_backup_ns_ok(
        self: &Arc<DataStore>,
        ns: BackupNamespace,
    ) -> Result<impl Iterator<Item = BackupNamespace> + 'static, Error> {
        let this = Arc::clone(self);
        Ok(
            ListNamespaces::new(Arc::clone(self), ns)?.filter_map(move |ns| match ns {
                Ok(ns) => Some(ns),
                Err(err) => {
                    log::error!("list groups error on datastore {} - {}", this.name(), err);
                    None
                }
            }),
        )
    }

    /// Get a streaming iter over single-level backup namespaces of a datatstore
    ///
    /// The iterated item is still a Result that can contain errors from rather unexptected FS or
    /// parsing errors.
    pub fn recursive_iter_backup_ns(
        self: &Arc<DataStore>,
        ns: BackupNamespace,
    ) -> Result<ListNamespacesRecursive, Error> {
        ListNamespacesRecursive::new(Arc::clone(self), ns)
    }

    /// Get a streaming iter over single-level backup namespaces of a datatstore, filtered by Ok
    ///
    /// The iterated item's result is already unwrapped, if it contained an error it will be
    /// logged. Can be useful in iterator chain commands
    pub fn recursive_iter_backup_ns_ok(
        self: &Arc<DataStore>,
        ns: BackupNamespace,
        max_depth: Option<usize>,
    ) -> Result<impl Iterator<Item = BackupNamespace> + 'static, Error> {
        let this = Arc::clone(self);
        Ok(if let Some(depth) = max_depth {
            ListNamespacesRecursive::new_max_depth(Arc::clone(self), ns, depth)?
        } else {
            ListNamespacesRecursive::new(Arc::clone(self), ns)?
        }
        .filter_map(move |ns| match ns {
            Ok(ns) => Some(ns),
            Err(err) => {
                log::error!("list groups error on datastore {} - {}", this.name(), err);
                None
            }
        }))
    }

    /// Get a streaming iter over top-level backup groups of a datatstore of a particular type.
    ///
    /// The iterated item is still a Result that can contain errors from rather unexptected FS or
    /// parsing errors.
    pub fn iter_backup_type(
        self: &Arc<DataStore>,
        ns: BackupNamespace,
        ty: BackupType,
    ) -> Result<ListGroupsType, Error> {
        ListGroupsType::new(Arc::clone(self), ns, ty)
    }

    /// Get a streaming iter over top-level backup groups of a datatstore of a particular type,
    /// filtered by `Ok` results
    ///
    /// The iterated item's result is already unwrapped, if it contained an error it will be
    /// logged. Can be useful in iterator chain commands
    pub fn iter_backup_type_ok(
        self: &Arc<DataStore>,
        ns: BackupNamespace,
        ty: BackupType,
    ) -> Result<impl Iterator<Item = BackupGroup> + 'static, Error> {
        Ok(self.iter_backup_type(ns, ty)?.ok())
    }

    /// Get a streaming iter over top-level backup groups of a datatstore
    ///
    /// The iterated item is still a Result that can contain errors from rather unexptected FS or
    /// parsing errors.
    pub fn iter_backup_groups(
        self: &Arc<DataStore>,
        ns: BackupNamespace,
    ) -> Result<ListGroups, Error> {
        ListGroups::new(Arc::clone(self), ns)
    }

    /// Get a streaming iter over top-level backup groups of a datatstore, filtered by Ok results
    ///
    /// The iterated item's result is already unwrapped, if it contained an error it will be
    /// logged. Can be useful in iterator chain commands
    pub fn iter_backup_groups_ok(
        self: &Arc<DataStore>,
        ns: BackupNamespace,
    ) -> Result<impl Iterator<Item = BackupGroup> + 'static, Error> {
        Ok(self.iter_backup_groups(ns)?.ok())
    }

    /// Get a in-memory vector for all top-level backup groups of a datatstore
    ///
    /// NOTE: using the iterator directly is most often more efficient w.r.t. memory usage
    pub fn list_backup_groups(
        self: &Arc<DataStore>,
        ns: BackupNamespace,
    ) -> Result<Vec<BackupGroup>, Error> {
        ListGroups::new(Arc::clone(self), ns)?.collect()
    }

    pub fn list_images(&self) -> Result<Vec<PathBuf>, Error> {
        let base = self.base_path();

        let mut list = vec![];

        use walkdir::WalkDir;

        let walker = WalkDir::new(base).into_iter();

        // make sure we skip .chunks (and other hidden files to keep it simple)
        fn is_hidden(entry: &walkdir::DirEntry) -> bool {
            entry
                .file_name()
                .to_str()
                .map(|s| s.starts_with('.'))
                .unwrap_or(false)
        }
        let handle_entry_err = |err: walkdir::Error| {
            // first, extract the actual IO error and the affected path
            let (inner, path) = match (err.io_error(), err.path()) {
                (None, _) => return Ok(()), // not an IO-error
                (Some(inner), Some(path)) => (inner, path),
                (Some(inner), None) => bail!("unexpected error on datastore traversal: {inner}"),
            };
            if inner.kind() == io::ErrorKind::PermissionDenied {
                if err.depth() <= 1 && path.ends_with("lost+found") {
                    // allow skipping of (root-only) ext4 fsck-directory on EPERM ..
                    return Ok(());
                }
                // .. but do not ignore EPERM in general, otherwise we might prune too many chunks.
                // E.g., if users messed up with owner/perms on a rsync
                bail!("cannot continue garbage-collection safely, permission denied on: {path:?}");
            } else if inner.kind() == io::ErrorKind::NotFound {
                log::info!("ignoring vanished file: {path:?}");
                return Ok(());
            } else {
                bail!("unexpected error on datastore traversal: {inner} - {path:?}");
            }
        };
        for entry in walker.filter_entry(|e| !is_hidden(e)) {
            let path = match entry {
                Ok(entry) => entry.into_path(),
                Err(err) => {
                    handle_entry_err(err)?;
                    continue;
                }
            };
            if let Ok(archive_type) = archive_type(&path) {
                if archive_type == ArchiveType::FixedIndex
                    || archive_type == ArchiveType::DynamicIndex
                {
                    list.push(path);
                }
            }
        }

        Ok(list)
    }

    // mark chunks  used by ``index`` as used
    fn index_mark_used_chunks<I: IndexFile>(
        &self,
        index: I,
        file_name: &Path, // only used for error reporting
        status: &mut GarbageCollectionStatus,
        worker: &dyn WorkerTaskContext,
    ) -> Result<(), Error> {
        status.index_file_count += 1;
        status.index_data_bytes += index.index_bytes();

        for pos in 0..index.index_count() {
            worker.check_abort()?;
            worker.fail_on_shutdown()?;
            let digest = index.index_digest(pos).unwrap();
            if !self.inner.chunk_store.cond_touch_chunk(digest, false)? {
                let hex = hex::encode(digest);
                task_warn!(
                    worker,
                    "warning: unable to access non-existent chunk {hex}, required by {file_name:?}"
                );

                // touch any corresponding .bad files to keep them around, meaning if a chunk is
                // rewritten correctly they will be removed automatically, as well as if no index
                // file requires the chunk anymore (won't get to this loop then)
                for i in 0..=9 {
                    let bad_ext = format!("{}.bad", i);
                    let mut bad_path = PathBuf::new();
                    bad_path.push(self.chunk_path(digest).0);
                    bad_path.set_extension(bad_ext);
                    self.inner.chunk_store.cond_touch_path(&bad_path, false)?;
                }
            }
        }
        Ok(())
    }

    fn mark_used_chunks(
        &self,
        status: &mut GarbageCollectionStatus,
        worker: &dyn WorkerTaskContext,
    ) -> Result<(), Error> {
        let image_list = self.list_images()?;
        let image_count = image_list.len();

        let mut last_percentage: usize = 0;

        let mut strange_paths_count: u64 = 0;

        for (i, img) in image_list.into_iter().enumerate() {
            worker.check_abort()?;
            worker.fail_on_shutdown()?;

            if let Some(backup_dir_path) = img.parent() {
                let backup_dir_path = backup_dir_path.strip_prefix(self.base_path())?;
                if let Some(backup_dir_str) = backup_dir_path.to_str() {
                    if pbs_api_types::parse_ns_and_snapshot(backup_dir_str).is_err() {
                        strange_paths_count += 1;
                    }
                }
            }

            match std::fs::File::open(&img) {
                Ok(file) => {
                    if let Ok(archive_type) = archive_type(&img) {
                        if archive_type == ArchiveType::FixedIndex {
                            let index = FixedIndexReader::new(file).map_err(|e| {
                                format_err!("can't read index '{}' - {}", img.to_string_lossy(), e)
                            })?;
                            self.index_mark_used_chunks(index, &img, status, worker)?;
                        } else if archive_type == ArchiveType::DynamicIndex {
                            let index = DynamicIndexReader::new(file).map_err(|e| {
                                format_err!("can't read index '{}' - {}", img.to_string_lossy(), e)
                            })?;
                            self.index_mark_used_chunks(index, &img, status, worker)?;
                        }
                    }
                }
                Err(err) if err.kind() == io::ErrorKind::NotFound => (), // ignore vanished files
                Err(err) => bail!("can't open index {} - {}", img.to_string_lossy(), err),
            }

            let percentage = (i + 1) * 100 / image_count;
            if percentage > last_percentage {
                task_log!(
                    worker,
                    "marked {}% ({} of {} index files)",
                    percentage,
                    i + 1,
                    image_count,
                );
                last_percentage = percentage;
            }
        }

        if strange_paths_count > 0 {
            task_log!(
                worker,
                "found (and marked) {} index files outside of expected directory scheme",
                strange_paths_count,
            );
        }

        Ok(())
    }

    pub fn last_gc_status(&self) -> GarbageCollectionStatus {
        self.inner.last_gc_status.lock().unwrap().clone()
    }

    pub fn garbage_collection_running(&self) -> bool {
        !matches!(self.inner.gc_mutex.try_lock(), Ok(_))
    }

    pub fn garbage_collection(
        &self,
        worker: &dyn WorkerTaskContext,
        upid: &UPID,
    ) -> Result<(), Error> {
        if let Ok(ref mut _mutex) = self.inner.gc_mutex.try_lock() {
            // avoids that we run GC if an old daemon process has still a
            // running backup writer, which is not save as we have no "oldest
            // writer" information and thus no safe atime cutoff
            let _exclusive_lock = self.inner.chunk_store.try_exclusive_lock()?;

            let phase1_start_time = proxmox_time::epoch_i64();
            let oldest_writer = self
                .inner
                .chunk_store
                .oldest_writer()
                .unwrap_or(phase1_start_time);

            let mut gc_status = GarbageCollectionStatus {
                upid: Some(upid.to_string()),
                ..Default::default()
            };

            task_log!(worker, "Start GC phase1 (mark used chunks)");

            self.mark_used_chunks(&mut gc_status, worker)?;

            task_log!(worker, "Start GC phase2 (sweep unused chunks)");
            self.inner.chunk_store.sweep_unused_chunks(
                oldest_writer,
                phase1_start_time,
                &mut gc_status,
                worker,
            )?;

            task_log!(
                worker,
                "Removed garbage: {}",
                HumanByte::from(gc_status.removed_bytes),
            );
            task_log!(worker, "Removed chunks: {}", gc_status.removed_chunks);
            if gc_status.pending_bytes > 0 {
                task_log!(
                    worker,
                    "Pending removals: {} (in {} chunks)",
                    HumanByte::from(gc_status.pending_bytes),
                    gc_status.pending_chunks,
                );
            }
            if gc_status.removed_bad > 0 {
                task_log!(worker, "Removed bad chunks: {}", gc_status.removed_bad);
            }

            if gc_status.still_bad > 0 {
                task_log!(worker, "Leftover bad chunks: {}", gc_status.still_bad);
            }

            task_log!(
                worker,
                "Original data usage: {}",
                HumanByte::from(gc_status.index_data_bytes),
            );

            if gc_status.index_data_bytes > 0 {
                let comp_per =
                    (gc_status.disk_bytes as f64 * 100.) / gc_status.index_data_bytes as f64;
                task_log!(
                    worker,
                    "On-Disk usage: {} ({:.2}%)",
                    HumanByte::from(gc_status.disk_bytes),
                    comp_per,
                );
            }

            task_log!(worker, "On-Disk chunks: {}", gc_status.disk_chunks);

            let deduplication_factor = if gc_status.disk_bytes > 0 {
                (gc_status.index_data_bytes as f64) / (gc_status.disk_bytes as f64)
            } else {
                1.0
            };

            task_log!(worker, "Deduplication factor: {:.2}", deduplication_factor);

            if gc_status.disk_chunks > 0 {
                let avg_chunk = gc_status.disk_bytes / (gc_status.disk_chunks as u64);
                task_log!(worker, "Average chunk size: {}", HumanByte::from(avg_chunk));
            }

            if let Ok(serialized) = serde_json::to_string(&gc_status) {
                let mut path = self.base_path();
                path.push(".gc-status");

                let backup_user = pbs_config::backup_user()?;
                let mode = nix::sys::stat::Mode::from_bits_truncate(0o0644);
                // set the correct owner/group/permissions while saving file
                // owner(rw) = backup, group(r)= backup
                let options = CreateOptions::new()
                    .perm(mode)
                    .owner(backup_user.uid)
                    .group(backup_user.gid);

                // ignore errors
                let _ = replace_file(path, serialized.as_bytes(), options, false);
            }

            *self.inner.last_gc_status.lock().unwrap() = gc_status;
        } else {
            bail!("Start GC failed - (already running/locked)");
        }

        Ok(())
    }

    pub fn try_shared_chunk_store_lock(&self) -> Result<ProcessLockSharedGuard, Error> {
        self.inner.chunk_store.try_shared_lock()
    }

    pub fn chunk_path(&self, digest: &[u8; 32]) -> (PathBuf, String) {
        self.inner.chunk_store.chunk_path(digest)
    }

    pub fn cond_touch_chunk(&self, digest: &[u8; 32], assert_exists: bool) -> Result<bool, Error> {
        self.inner
            .chunk_store
            .cond_touch_chunk(digest, assert_exists)
    }

    pub fn insert_chunk(&self, chunk: &DataBlob, digest: &[u8; 32]) -> Result<(bool, u64), Error> {
        self.inner.chunk_store.insert_chunk(chunk, digest)
    }

    pub fn stat_chunk(&self, digest: &[u8; 32]) -> Result<std::fs::Metadata, Error> {
        let (chunk_path, _digest_str) = self.inner.chunk_store.chunk_path(digest);
        std::fs::metadata(chunk_path).map_err(Error::from)
    }

    pub fn load_chunk(&self, digest: &[u8; 32]) -> Result<DataBlob, Error> {
        let (chunk_path, digest_str) = self.inner.chunk_store.chunk_path(digest);

        proxmox_lang::try_block!({
            let mut file = std::fs::File::open(&chunk_path)?;
            DataBlob::load_from_reader(&mut file)
        })
        .map_err(|err| {
            format_err!(
                "store '{}', unable to load chunk '{}' - {}",
                self.name(),
                digest_str,
                err,
            )
        })
    }

    /// Updates the protection status of the specified snapshot.
    pub fn update_protection(&self, backup_dir: &BackupDir, protection: bool) -> Result<(), Error> {
        let full_path = backup_dir.full_path();

        if !full_path.exists() {
            bail!("snapshot {} does not exist!", backup_dir.dir());
        }

        let _guard = lock_dir_noblock(&full_path, "snapshot", "possibly running or in use")?;

        let protected_path = backup_dir.protected_file();
        if protection {
            std::fs::File::create(protected_path)
                .map_err(|err| format_err!("could not create protection file: {}", err))?;
        } else if let Err(err) = std::fs::remove_file(protected_path) {
            // ignore error for non-existing file
            if err.kind() != std::io::ErrorKind::NotFound {
                bail!("could not remove protection file: {}", err);
            }
        }

        Ok(())
    }

    pub fn verify_new(&self) -> bool {
        self.inner.verify_new
    }

    /// returns a list of chunks sorted by their inode number on disk chunks that couldn't get
    /// stat'ed are placed at the end of the list
    pub fn get_chunks_in_order<F, A>(
        &self,
        index: &(dyn IndexFile + Send),
        skip_chunk: F,
        check_abort: A,
    ) -> Result<Vec<(usize, u64)>, Error>
    where
        F: Fn(&[u8; 32]) -> bool,
        A: Fn(usize) -> Result<(), Error>,
    {
        let index_count = index.index_count();
        let mut chunk_list = Vec::with_capacity(index_count);
        use std::os::unix::fs::MetadataExt;
        for pos in 0..index_count {
            check_abort(pos)?;

            let info = index.chunk_info(pos).unwrap();

            if skip_chunk(&info.digest) {
                continue;
            }

            let ino = match self.inner.chunk_order {
                ChunkOrder::Inode => {
                    match self.stat_chunk(&info.digest) {
                        Err(_) => u64::MAX, // could not stat, move to end of list
                        Ok(metadata) => metadata.ino(),
                    }
                }
                ChunkOrder::None => 0,
            };

            chunk_list.push((pos, ino));
        }

        match self.inner.chunk_order {
            // sorting by inode improves data locality, which makes it lots faster on spinners
            ChunkOrder::Inode => {
                chunk_list.sort_unstable_by(|(_, ino_a), (_, ino_b)| ino_a.cmp(ino_b))
            }
            ChunkOrder::None => {}
        }

        Ok(chunk_list)
    }

    /// Open a backup group from this datastore.
    pub fn backup_group(
        self: &Arc<Self>,
        ns: BackupNamespace,
        group: pbs_api_types::BackupGroup,
    ) -> BackupGroup {
        BackupGroup::new(Arc::clone(self), ns, group)
    }

    /// Open a backup group from this datastore.
    pub fn backup_group_from_parts<T>(
        self: &Arc<Self>,
        ns: BackupNamespace,
        ty: BackupType,
        id: T,
    ) -> BackupGroup
    where
        T: Into<String>,
    {
        self.backup_group(ns, (ty, id.into()).into())
    }

    /*
    /// Open a backup group from this datastore by backup group path such as `vm/100`.
    ///
    /// Convenience method for `store.backup_group(path.parse()?)`
    pub fn backup_group_from_path(self: &Arc<Self>, path: &str) -> Result<BackupGroup, Error> {
        todo!("split out the namespace");
    }
    */

    /// Open a snapshot (backup directory) from this datastore.
    pub fn backup_dir(
        self: &Arc<Self>,
        ns: BackupNamespace,
        dir: pbs_api_types::BackupDir,
    ) -> Result<BackupDir, Error> {
        BackupDir::with_group(self.backup_group(ns, dir.group), dir.time)
    }

    /// Open a snapshot (backup directory) from this datastore.
    pub fn backup_dir_from_parts<T>(
        self: &Arc<Self>,
        ns: BackupNamespace,
        ty: BackupType,
        id: T,
        time: i64,
    ) -> Result<BackupDir, Error>
    where
        T: Into<String>,
    {
        self.backup_dir(ns, (ty, id.into(), time).into())
    }

    /// Open a snapshot (backup directory) from this datastore with a cached rfc3339 time string.
    pub fn backup_dir_with_rfc3339<T: Into<String>>(
        self: &Arc<Self>,
        group: BackupGroup,
        time_string: T,
    ) -> Result<BackupDir, Error> {
        BackupDir::with_rfc3339(group, time_string.into())
    }

    /*
    /// Open a snapshot (backup directory) from this datastore by a snapshot path.
    pub fn backup_dir_from_path(self: &Arc<Self>, path: &str) -> Result<BackupDir, Error> {
        todo!("split out the namespace");
    }
    */

    /// Syncs the filesystem of the datastore if 'sync_level' is set to
    /// [`DatastoreFSyncLevel::Filesystem`]. Uses syncfs(2).
    pub fn try_ensure_sync_level(&self) -> Result<(), Error> {
        if self.inner.sync_level != DatastoreFSyncLevel::Filesystem {
            return Ok(());
        }
        let file = std::fs::File::open(self.base_path())?;
        let fd = file.as_raw_fd();
        log::info!("syncing filesystem");
        if unsafe { libc::syncfs(fd) } < 0 {
            bail!("error during syncfs: {}", std::io::Error::last_os_error());
        }
        Ok(())
    }

    /// Destroy a datastore. This requires that there are no active operations on the datastore.
    ///
    /// This is a synchronous operation and should be run in a worker-thread.
    pub fn destroy(
        name: &str,
        destroy_data: bool,
        worker: &dyn WorkerTaskContext,
    ) -> Result<(), Error> {
        let config_lock = pbs_config::datastore::lock_config()?;

        let (mut config, _digest) = pbs_config::datastore::config()?;
        let mut datastore_config: DataStoreConfig = config.lookup("datastore", name)?;

        datastore_config.maintenance_mode = Some("type=delete".to_string());
        config.set_data(name, "datastore", &datastore_config)?;
        pbs_config::datastore::save_config(&config)?;
        drop(config_lock);

        let (operations, _lock) = task_tracking::get_active_operations_locked(name)?;

        if operations.read != 0 || operations.write != 0 {
            bail!("datastore is currently in use");
        }

        let base = PathBuf::from(&datastore_config.path);

        let mut ok = true;
        if destroy_data {
            let remove = |subdir, ok: &mut bool| {
                if let Err(err) = std::fs::remove_dir_all(base.join(subdir)) {
                    if err.kind() != io::ErrorKind::NotFound {
                        task_warn!(worker, "failed to remove {subdir:?} subdirectory: {err}");
                        *ok = false;
                    }
                }
            };

            task_log!(worker, "Deleting datastore data...");
            remove("ns", &mut ok); // ns first
            remove("ct", &mut ok);
            remove("vm", &mut ok);
            remove("host", &mut ok);

            if ok {
                if let Err(err) = std::fs::remove_file(base.join(".gc-status")) {
                    if err.kind() != io::ErrorKind::NotFound {
                        task_warn!(worker, "failed to remove .gc-status file: {err}");
                        ok = false;
                    }
                }
            }

            // chunks get removed last and only if the backups were successfully deleted
            if ok {
                remove(".chunks", &mut ok);
            }
        }

        // now the config
        if ok {
            task_log!(worker, "Removing datastore from config...");
            let _lock = pbs_config::datastore::lock_config()?;
            let _ = config.sections.remove(name);
            pbs_config::datastore::save_config(&config)?;
        }

        // finally the lock & toplevel directory
        if destroy_data {
            if ok {
                if let Err(err) = std::fs::remove_file(base.join(".lock")) {
                    if err.kind() != io::ErrorKind::NotFound {
                        task_warn!(worker, "failed to remove .lock file: {err}");
                        ok = false;
                    }
                }
            }

            if ok {
                task_log!(worker, "Finished deleting data.");

                match std::fs::remove_dir(base) {
                    Ok(()) => task_log!(worker, "Removed empty datastore directory."),
                    Err(err) if err.kind() == io::ErrorKind::NotFound => {
                        // weird, but ok
                    }
                    Err(err) if err.is_errno(nix::errno::Errno::EBUSY) => {
                        task_warn!(
                            worker,
                            "Cannot delete datastore directory (is it a mount point?)."
                        )
                    }
                    Err(err) if err.is_errno(nix::errno::Errno::ENOTEMPTY) => {
                        task_warn!(worker, "Datastore directory not empty, not deleting.")
                    }
                    Err(err) => {
                        task_warn!(worker, "Failed to remove datastore directory: {err}");
                    }
                }
            } else {
                task_log!(worker, "There were errors deleting data.");
            }
        }

        Ok(())
    }
}
