use failure::*;

use std::io;
use std::path::{PathBuf, Path};
use std::collections::HashMap;
use lazy_static::lazy_static;
use std::sync::{Mutex, Arc};

use crate::tools;
use crate::config::datastore;
use super::chunk_store::*;
use super::fixed_index::*;
use super::dynamic_index::*;
use super::index::*;
use super::backup_info::*;
use crate::server::WorkerTask;

lazy_static!{
    static ref DATASTORE_MAP: Mutex<HashMap<String, Arc<DataStore>>> =  Mutex::new(HashMap::new());
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

        let config = datastore::config()?;
        let (_, store_config) = config.sections.get(name)
            .ok_or(format_err!("no such datastore '{}'", name))?;

        let path = store_config["path"].as_str().unwrap();

        let mut map = DATASTORE_MAP.lock().unwrap();

        if let Some(datastore) = map.get(name) {
            // Compare Config - if changed, create new Datastore object!
            if datastore.chunk_store.base == PathBuf::from(path) {
                return Ok(datastore.clone());
            }
        }

        let datastore = DataStore::open(name)?;

        let datastore = Arc::new(datastore);
        map.insert(name.to_string(), datastore.clone());

        Ok(datastore)
    }

    pub fn open(store_name: &str) -> Result<Self, Error> {

        let config = datastore::config()?;
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
        print_percentage: bool,
    ) -> Result<
        impl Iterator<Item = Result<tools::fs::ReadDirEntry, Error>>,
        Error
    > {
        self.chunk_store.get_chunk_iterator(print_percentage)
    }

    pub fn create_fixed_writer<P: AsRef<Path>>(&self, filename: P, size: usize, chunk_size: usize) -> Result<FixedIndexWriter, Error> {

        let index = FixedIndexWriter::create(self.chunk_store.clone(), filename.as_ref(), size, chunk_size)?;

        Ok(index)
    }

    pub fn open_fixed_reader<P: AsRef<Path>>(&self, filename: P) -> Result<FixedIndexReader, Error> {

        let index = FixedIndexReader::open(self.chunk_store.clone(), filename.as_ref())?;

        Ok(index)
    }

    pub fn create_dynamic_writer<P: AsRef<Path>>(
        &self, filename: P,
        chunk_size: usize
    ) -> Result<DynamicIndexWriter, Error> {

        let index = DynamicIndexWriter::create(
            self.chunk_store.clone(), filename.as_ref(), chunk_size)?;

        Ok(index)
    }

    pub fn open_dynamic_reader<P: AsRef<Path>>(&self, filename: P) -> Result<DynamicIndexReader, Error> {

        let index = DynamicIndexReader::open(self.chunk_store.clone(), filename.as_ref())?;

        Ok(index)
    }

    pub fn open_index<P>(&self, filename: P) -> Result<Box<dyn IndexFile + Send>, Error>
    where
        P: AsRef<Path>,
    {
        let filename = filename.as_ref();
        let out: Box<dyn IndexFile + Send> =
            match filename.extension().and_then(|ext| ext.to_str()) {
                Some("didx") => Box::new(self.open_dynamic_reader(filename)?),
                Some("fidx") => Box::new(self.open_fixed_reader(filename)?),
                _ => bail!("cannot open index file of unknown type: {:?}", filename),
            };
        Ok(out)
    }

    pub fn base_path(&self) -> PathBuf {
        self.chunk_store.base_path()
    }

    /// Remove a backup directory including all content
    pub fn remove_backup_dir(&self, backup_dir: &BackupDir,
    ) ->  Result<(), io::Error> {

        let relative_path = backup_dir.relative_path();
        let mut full_path = self.base_path();
        full_path.push(&relative_path);

        log::info!("removing backup {:?}", full_path);
        std::fs::remove_dir_all(full_path)?;

        Ok(())
    }

    pub fn create_backup_dir(&self, backup_dir: &BackupDir) ->  Result<(PathBuf, bool), io::Error> {

        // create intermediate path first:
        let mut full_path = self.base_path();
        full_path.push(backup_dir.group().group_path());
        std::fs::create_dir_all(&full_path)?;

        let relative_path = backup_dir.relative_path();
        let mut full_path = self.base_path();
        full_path.push(&relative_path);

        // create the last component now
        match std::fs::create_dir(&full_path) {
            Ok(_) => Ok((relative_path, true)),
            Err(ref e) if e.kind() == io::ErrorKind::AlreadyExists => Ok((relative_path, false)),
            Err(e) => Err(e)
        }
    }

    /// Finds the latest backup inside a backup group
    pub fn last_backup(&self, backup_type: &str, backup_id: &str) -> Result<Option<BackupInfo>, Error> {
        let group = BackupGroup::new(backup_type, backup_id);

        let backups = group.list_backups(&self.base_path())?;

        Ok(backups.into_iter().max_by_key(|item| item.backup_dir.backup_time()))
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

        for entry in walker.filter_entry(|e| !is_hidden(e)) {
            let path = entry?.into_path();
            if let Some(ext) = path.extension() {
                if ext == "fidx" {
                    list.push(path);
                } else if ext == "didx" {
                    list.push(path);
                }
            }
        }

        Ok(list)
    }

    fn mark_used_chunks(&self, status: &mut GarbageCollectionStatus) -> Result<(), Error> {

        let image_list = self.list_images()?;

        for path in image_list {

            tools::fail_on_shutdown()?;

            if let Some(ext) = path.extension() {
                if ext == "fidx" {
                    let index = self.open_fixed_reader(&path)?;
                    index.mark_used_chunks(status)?;
                } else if ext == "didx" {
                    let index = self.open_dynamic_reader(&path)?;
                    index.mark_used_chunks(status)?;
                }
            }
        }

        Ok(())
    }

    pub fn last_gc_status(&self) -> GarbageCollectionStatus {
        self.last_gc_status.lock().unwrap().clone()
    }

    pub fn garbage_collection(&self, worker: Arc<WorkerTask>) -> Result<(), Error> {

        if let Ok(ref mut _mutex) = self.gc_mutex.try_lock() {

            let _exclusive_lock =  self.chunk_store.try_exclusive_lock()?;

            let oldest_writer = self.chunk_store.oldest_writer();

            let mut gc_status = GarbageCollectionStatus::default();
            gc_status.upid = Some(worker.to_string());

            worker.log("Start GC phase1 (mark chunks)");

            self.mark_used_chunks(&mut gc_status)?;

            worker.log("Start GC phase2 (sweep unused chunks)");
            self.chunk_store.sweep_unused_chunks(oldest_writer, &mut gc_status)?;

            worker.log(&format!("Used bytes: {}", gc_status.used_bytes));
            worker.log(&format!("Used chunks: {}", gc_status.used_chunks));
            worker.log(&format!("Disk bytes: {}", gc_status.disk_bytes));
            worker.log(&format!("Disk chunks: {}", gc_status.disk_chunks));

            *self.last_gc_status.lock().unwrap() = gc_status;

        } else {
            bail!("Start GC failed - (already running/locked)");
        }

        Ok(())
    }

    pub fn insert_chunk(&self, chunk: &[u8]) -> Result<(bool, [u8; 32], u64), Error> {
        self.chunk_store.insert_chunk(chunk)
    }

    pub fn insert_chunk_noverify(
        &self,
        digest: &[u8; 32],
        chunk: &[u8],
    ) -> Result<(bool, u64), Error> {
        self.chunk_store.insert_chunk_noverify(digest, chunk)
    }
}
