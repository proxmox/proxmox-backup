use std::collections::{HashSet, HashMap};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::convert::TryFrom;

use anyhow::{bail, format_err, Error};
use lazy_static::lazy_static;
use chrono::{DateTime, Utc};
use serde_json::Value;

use proxmox::tools::fs::{replace_file, CreateOptions};

use super::backup_info::{BackupGroup, BackupDir};
use super::chunk_store::ChunkStore;
use super::dynamic_index::{DynamicIndexReader, DynamicIndexWriter};
use super::fixed_index::{FixedIndexReader, FixedIndexWriter};
use super::manifest::{MANIFEST_BLOB_NAME, CLIENT_LOG_BLOB_NAME, BackupManifest};
use super::index::*;
use super::{DataBlob, ArchiveType, archive_type};
use crate::config::datastore;
use crate::server::WorkerTask;
use crate::tools;
use crate::tools::format::HumanByte;
use crate::tools::fs::{lock_dir_noblock, DirLockGuard};
use crate::api2::types::{GarbageCollectionStatus, Userid};

lazy_static! {
    static ref DATASTORE_MAP: Mutex<HashMap<String, Arc<DataStore>>> = Mutex::new(HashMap::new());
}

/// Datastore Management
///
/// A Datastore can store severals backups, and provides the
/// management interface for backup.
pub struct DataStore {
    chunk_store: Arc<ChunkStore>,
    gc_mutex: Mutex<bool>,
    last_gc_status: Mutex<GarbageCollectionStatus>,
}

impl DataStore {

    pub fn lookup_datastore(name: &str) -> Result<Arc<DataStore>, Error> {

        let (config, _digest) = datastore::config()?;
        let config: datastore::DataStoreConfig = config.lookup("datastore", name)?;

        let mut map = DATASTORE_MAP.lock().unwrap();

        if let Some(datastore) = map.get(name) {
            // Compare Config - if changed, create new Datastore object!
            if datastore.chunk_store.base == PathBuf::from(&config.path) {
                return Ok(datastore.clone());
            }
        }

        let datastore = DataStore::open(name)?;

        let datastore = Arc::new(datastore);
        map.insert(name.to_string(), datastore.clone());

        Ok(datastore)
    }

    pub fn open(store_name: &str) -> Result<Self, Error> {

        let (config, _digest) = datastore::config()?;
        let (_, store_config) = config.sections.get(store_name)
            .ok_or(format_err!("no such datastore '{}'", store_name))?;

        let path = store_config["path"].as_str().unwrap();

        let chunk_store = ChunkStore::open(store_name, path)?;

        let gc_status = GarbageCollectionStatus::default();

        Ok(Self {
            chunk_store: Arc::new(chunk_store),
            gc_mutex: Mutex::new(false),
            last_gc_status: Mutex::new(gc_status),
        })
    }

    pub fn get_chunk_iterator(
        &self,
    ) -> Result<
        impl Iterator<Item = (Result<tools::fs::ReadDirEntry, Error>, usize)>,
        Error
    > {
        self.chunk_store.get_chunk_iterator()
    }

    pub fn create_fixed_writer<P: AsRef<Path>>(&self, filename: P, size: usize, chunk_size: usize) -> Result<FixedIndexWriter, Error> {

        let index = FixedIndexWriter::create(self.chunk_store.clone(), filename.as_ref(), size, chunk_size)?;

        Ok(index)
    }

    pub fn open_fixed_reader<P: AsRef<Path>>(&self, filename: P) -> Result<FixedIndexReader, Error> {

        let full_path =  self.chunk_store.relative_path(filename.as_ref());

        let index = FixedIndexReader::open(&full_path)?;

        Ok(index)
    }

    pub fn create_dynamic_writer<P: AsRef<Path>>(
        &self, filename: P,
    ) -> Result<DynamicIndexWriter, Error> {

        let index = DynamicIndexWriter::create(
            self.chunk_store.clone(), filename.as_ref())?;

        Ok(index)
    }

    pub fn open_dynamic_reader<P: AsRef<Path>>(&self, filename: P) -> Result<DynamicIndexReader, Error> {

        let full_path =  self.chunk_store.relative_path(filename.as_ref());

        let index = DynamicIndexReader::open(&full_path)?;

        Ok(index)
    }

    pub fn open_index<P>(&self, filename: P) -> Result<Box<dyn IndexFile + Send>, Error>
    where
        P: AsRef<Path>,
    {
        let filename = filename.as_ref();
        let out: Box<dyn IndexFile + Send> =
            match archive_type(filename)? {
                ArchiveType::DynamicIndex => Box::new(self.open_dynamic_reader(filename)?),
                ArchiveType::FixedIndex => Box::new(self.open_fixed_reader(filename)?),
                _ => bail!("cannot open index file of unknown type: {:?}", filename),
            };
        Ok(out)
    }

    pub fn name(&self) -> &str {
        self.chunk_store.name()
    }

    pub fn base_path(&self) -> PathBuf {
        self.chunk_store.base_path()
    }

    /// Cleanup a backup directory
    ///
    /// Removes all files not mentioned in the manifest.
    pub fn cleanup_backup_dir(&self, backup_dir: &BackupDir, manifest: &BackupManifest
    ) ->  Result<(), Error> {

        let mut full_path = self.base_path();
        full_path.push(backup_dir.relative_path());

        let mut wanted_files = HashSet::new();
        wanted_files.insert(MANIFEST_BLOB_NAME.to_string());
        wanted_files.insert(CLIENT_LOG_BLOB_NAME.to_string());
        manifest.files().iter().for_each(|item| { wanted_files.insert(item.filename.clone()); });

        for item in tools::fs::read_subdir(libc::AT_FDCWD, &full_path)? {
            if let Ok(item) = item {
                if let Some(file_type) = item.file_type() {
                    if file_type != nix::dir::Type::File { continue; }
                }
                let file_name = item.file_name().to_bytes();
                if file_name == b"." || file_name == b".." { continue; };

                if let Ok(name) = std::str::from_utf8(file_name) {
                    if wanted_files.contains(name) { continue; }
                }
                println!("remove unused file {:?}", item.file_name());
                let dirfd = item.parent_fd();
                let _res = unsafe { libc::unlinkat(dirfd, item.file_name().as_ptr(), 0) };
            }
        }

        Ok(())
    }

    /// Returns the absolute path for a backup_group
    pub fn group_path(&self, backup_group: &BackupGroup) -> PathBuf {
        let mut full_path = self.base_path();
        full_path.push(backup_group.group_path());
        full_path
    }

    /// Returns the absolute path for backup_dir
    pub fn snapshot_path(&self, backup_dir: &BackupDir) -> PathBuf {
        let mut full_path = self.base_path();
        full_path.push(backup_dir.relative_path());
        full_path
    }

    /// Remove a complete backup group including all snapshots
    pub fn remove_backup_group(&self, backup_group: &BackupGroup) ->  Result<(), Error> {

        let full_path = self.group_path(backup_group);

        let _guard = tools::fs::lock_dir_noblock(&full_path, "backup group", "possible running backup")?;

        log::info!("removing backup group {:?}", full_path);
        std::fs::remove_dir_all(&full_path)
            .map_err(|err| {
                format_err!(
                    "removing backup group {:?} failed - {}",
                    full_path,
                    err,
                )
            })?;

        Ok(())
    }

    /// Remove a backup directory including all content
    pub fn remove_backup_dir(&self, backup_dir: &BackupDir, force: bool) ->  Result<(), Error> {

        let full_path = self.snapshot_path(backup_dir);

        let _guard;
        if !force {
            _guard = lock_dir_noblock(&full_path, "snapshot", "possibly running or used as base")?;
        }

        log::info!("removing backup snapshot {:?}", full_path);
        std::fs::remove_dir_all(&full_path)
            .map_err(|err| {
                format_err!(
                    "removing backup snapshot {:?} failed - {}",
                    full_path,
                    err,
                )
            })?;

        Ok(())
    }

    /// Returns the time of the last successful backup
    ///
    /// Or None if there is no backup in the group (or the group dir does not exist).
    pub fn last_successful_backup(&self, backup_group: &BackupGroup) -> Result<Option<DateTime<Utc>>, Error> {
        let base_path = self.base_path();
        let mut group_path = base_path.clone();
        group_path.push(backup_group.group_path());

        if group_path.exists() {
            backup_group.last_successful_backup(&base_path)
        } else {
            Ok(None)
        }
    }

    /// Returns the backup owner.
    ///
    /// The backup owner is the user who first created the backup group.
    pub fn get_owner(&self, backup_group: &BackupGroup) -> Result<Userid, Error> {
        let mut full_path = self.base_path();
        full_path.push(backup_group.group_path());
        full_path.push("owner");
        let owner = proxmox::tools::fs::file_read_firstline(full_path)?;
        Ok(owner.trim_end().parse()?) // remove trailing newline
    }

    /// Set the backup owner.
    pub fn set_owner(
        &self,
        backup_group: &BackupGroup,
        userid: &Userid,
        force: bool,
    ) -> Result<(), Error> {
        let mut path = self.base_path();
        path.push(backup_group.group_path());
        path.push("owner");

        let mut open_options = std::fs::OpenOptions::new();
        open_options.write(true);
        open_options.truncate(true);

        if force {
            open_options.create(true);
        } else {
            open_options.create_new(true);
        }

        let mut file = open_options.open(&path)
            .map_err(|err| format_err!("unable to create owner file {:?} - {}", path, err))?;

        write!(file, "{}\n", userid)
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
        backup_group: &BackupGroup,
        userid: &Userid,
    ) -> Result<(Userid, DirLockGuard), Error> {
        // create intermediate path first:
        let base_path = self.base_path();

        let mut full_path = base_path.clone();
        full_path.push(backup_group.backup_type());
        std::fs::create_dir_all(&full_path)?;

        full_path.push(backup_group.backup_id());

        // create the last component now
        match std::fs::create_dir(&full_path) {
            Ok(_) => {
                let guard = lock_dir_noblock(&full_path, "backup group", "another backup is already running")?;
                self.set_owner(backup_group, userid, false)?;
                let owner = self.get_owner(backup_group)?; // just to be sure
                Ok((owner, guard))
            }
            Err(ref err) if err.kind() == io::ErrorKind::AlreadyExists => {
                let guard = lock_dir_noblock(&full_path, "backup group", "another backup is already running")?;
                let owner = self.get_owner(backup_group)?; // just to be sure
                Ok((owner, guard))
            }
            Err(err) => bail!("unable to create backup group {:?} - {}", full_path, err),
        }
    }

    /// Creates a new backup snapshot inside a BackupGroup
    ///
    /// The BackupGroup directory needs to exist.
    pub fn create_locked_backup_dir(&self, backup_dir: &BackupDir)
        -> Result<(PathBuf, bool, DirLockGuard), Error>
    {
        let relative_path = backup_dir.relative_path();
        let mut full_path = self.base_path();
        full_path.push(&relative_path);

        let lock = ||
            lock_dir_noblock(&full_path, "snapshot", "internal error - tried creating snapshot that's already in use");

        match std::fs::create_dir(&full_path) {
            Ok(_) => Ok((relative_path, true, lock()?)),
            Err(ref e) if e.kind() == io::ErrorKind::AlreadyExists => Ok((relative_path, false, lock()?)),
            Err(e) => Err(e.into())
        }
    }

    pub fn list_images(&self) -> Result<Vec<PathBuf>, Error> {
        let base = self.base_path();

        let mut list = vec![];

        use walkdir::WalkDir;

        let walker = WalkDir::new(&base).same_file_system(true).into_iter();

        // make sure we skip .chunks (and other hidden files to keep it simple)
        fn is_hidden(entry: &walkdir::DirEntry) -> bool {
            entry.file_name()
                .to_str()
                .map(|s| s.starts_with("."))
                .unwrap_or(false)
        }
        let handle_entry_err = |err: walkdir::Error| {
            if let Some(inner) = err.io_error() {
                let path = err.path().unwrap_or(Path::new(""));
                match inner.kind() {
                    io::ErrorKind::PermissionDenied => {
                        // only allow to skip ext4 fsck directory, avoid GC if, for example,
                        // a user got file permissions wrong on datastore rsync to new server
                        if err.depth() > 1 || !path.ends_with("lost+found") {
                            bail!("cannot continue garbage-collection safely, permission denied on: {}", path.display())
                        }
                    },
                    _ => bail!("unexpected error on datastore traversal: {} - {}", inner, path.display()),
                }
            }
            Ok(())
        };
        for entry in walker.filter_entry(|e| !is_hidden(e)) {
            let path = match entry {
                Ok(entry) => entry.into_path(),
                Err(err) => {
                    handle_entry_err(err)?;
                    continue
                },
            };
            if let Ok(archive_type) = archive_type(&path) {
                if archive_type == ArchiveType::FixedIndex || archive_type == ArchiveType::DynamicIndex {
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
        worker: &WorkerTask,
    ) -> Result<(), Error> {

        status.index_file_count += 1;
        status.index_data_bytes += index.index_bytes();

        for pos in 0..index.index_count() {
            worker.fail_on_abort()?;
            tools::fail_on_shutdown()?;
            let digest = index.index_digest(pos).unwrap();
            if let Err(err) = self.chunk_store.touch_chunk(digest) {
                worker.warn(&format!("warning: unable to access chunk {}, required by {:?} - {}",
                      proxmox::tools::digest_to_hex(digest), file_name, err));
            }
        }
        Ok(())
    }

    fn mark_used_chunks(&self, status: &mut GarbageCollectionStatus, worker: &WorkerTask) -> Result<(), Error> {

        let image_list = self.list_images()?;

        for path in image_list {

            worker.fail_on_abort()?;
            tools::fail_on_shutdown()?;

            if let Ok(archive_type) = archive_type(&path) {
                if archive_type == ArchiveType::FixedIndex {
                    let index = self.open_fixed_reader(&path)?;
                    self.index_mark_used_chunks(index, &path, status, worker)?;
                } else if archive_type == ArchiveType::DynamicIndex {
                    let index = self.open_dynamic_reader(&path)?;
                    self.index_mark_used_chunks(index, &path, status, worker)?;
                }
            }
        }

        Ok(())
    }

    pub fn last_gc_status(&self) -> GarbageCollectionStatus {
        self.last_gc_status.lock().unwrap().clone()
    }

    pub fn garbage_collection_running(&self) -> bool {
        if let Ok(_) = self.gc_mutex.try_lock() { false } else { true }
    }

    pub fn garbage_collection(&self, worker: &WorkerTask) -> Result<(), Error> {

        if let Ok(ref mut _mutex) = self.gc_mutex.try_lock() {

            let _exclusive_lock =  self.chunk_store.try_exclusive_lock()?;

            let phase1_start_time = unsafe { libc::time(std::ptr::null_mut()) };
            let oldest_writer = self.chunk_store.oldest_writer().unwrap_or(phase1_start_time);

            let mut gc_status = GarbageCollectionStatus::default();
            gc_status.upid = Some(worker.to_string());

            worker.log("Start GC phase1 (mark used chunks)");

            self.mark_used_chunks(&mut gc_status, &worker)?;

            worker.log("Start GC phase2 (sweep unused chunks)");
            self.chunk_store.sweep_unused_chunks(oldest_writer, phase1_start_time, &mut gc_status, &worker)?;

            worker.log(&format!("Removed garbage: {}", HumanByte::from(gc_status.removed_bytes)));
            worker.log(&format!("Removed chunks: {}", gc_status.removed_chunks));
            if gc_status.pending_bytes > 0 {
                worker.log(&format!("Pending removals: {} (in {} chunks)", HumanByte::from(gc_status.pending_bytes), gc_status.pending_chunks));
            }

            worker.log(&format!("Original data usage: {}", HumanByte::from(gc_status.index_data_bytes)));

            if gc_status.index_data_bytes > 0 {
                let comp_per = (gc_status.disk_bytes as f64 * 100.)/gc_status.index_data_bytes as f64;
                worker.log(&format!("On-Disk usage: {} ({:.2}%)", HumanByte::from(gc_status.disk_bytes), comp_per));
            }

            worker.log(&format!("On-Disk chunks: {}", gc_status.disk_chunks));

            if gc_status.disk_chunks > 0 {
                let avg_chunk = gc_status.disk_bytes/(gc_status.disk_chunks as u64);
                worker.log(&format!("Average chunk size: {}", HumanByte::from(avg_chunk)));
            }

            *self.last_gc_status.lock().unwrap() = gc_status;

        } else {
            bail!("Start GC failed - (already running/locked)");
        }

        Ok(())
    }

    pub fn try_shared_chunk_store_lock(&self) -> Result<tools::ProcessLockSharedGuard, Error> {
        self.chunk_store.try_shared_lock()
    }

    pub fn chunk_path(&self, digest:&[u8; 32]) -> (PathBuf, String) {
        self.chunk_store.chunk_path(digest)
    }

    pub fn cond_touch_chunk(&self, digest: &[u8; 32], fail_if_not_exist: bool) -> Result<bool, Error> {
        self.chunk_store.cond_touch_chunk(digest, fail_if_not_exist)
    }

    pub fn insert_chunk(
        &self,
        chunk: &DataBlob,
        digest: &[u8; 32],
    ) -> Result<(bool, u64), Error> {
        self.chunk_store.insert_chunk(chunk, digest)
    }

    pub fn load_blob(&self, backup_dir: &BackupDir, filename: &str) -> Result<DataBlob, Error> {
        let mut path = self.base_path();
        path.push(backup_dir.relative_path());
        path.push(filename);

        proxmox::try_block!({
            let mut file = std::fs::File::open(&path)?;
            DataBlob::load_from_reader(&mut file)
        }).map_err(|err| format_err!("unable to load blob '{:?}' - {}", path, err))
    }


    pub fn load_chunk(&self, digest: &[u8; 32]) -> Result<DataBlob, Error> {

        let (chunk_path, digest_str) = self.chunk_store.chunk_path(digest);

        proxmox::try_block!({
            let mut file = std::fs::File::open(&chunk_path)?;
            DataBlob::load_from_reader(&mut file)
        }).map_err(|err| format_err!(
            "store '{}', unable to load chunk '{}' - {}",
            self.name(),
            digest_str,
            err,
        ))
     }

    pub fn load_manifest(
        &self,
        backup_dir: &BackupDir,
    ) -> Result<(BackupManifest, u64), Error> {
        let blob = self.load_blob(backup_dir, MANIFEST_BLOB_NAME)?;
        let raw_size = blob.raw_size();
        let manifest = BackupManifest::try_from(blob)?;
        Ok((manifest, raw_size))
    }

    pub fn load_manifest_json(
        &self,
        backup_dir: &BackupDir,
    ) -> Result<Value, Error> {
        let blob = self.load_blob(backup_dir, MANIFEST_BLOB_NAME)?;
        // no expected digest available
        let manifest_data = blob.decode(None, None)?;
        let manifest: Value = serde_json::from_slice(&manifest_data[..])?;
        Ok(manifest)
    }

    pub fn store_manifest(
        &self,
        backup_dir: &BackupDir,
        manifest: Value,
    ) -> Result<(), Error> {
        let manifest = serde_json::to_string_pretty(&manifest)?;
        let blob = DataBlob::encode(manifest.as_bytes(), None, true)?;
        let raw_data = blob.raw_data();

        let mut path = self.base_path();
        path.push(backup_dir.relative_path());
        path.push(MANIFEST_BLOB_NAME);

        replace_file(&path, raw_data, CreateOptions::new())?;

        Ok(())
    }
}
