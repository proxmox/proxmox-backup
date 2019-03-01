use failure::*;

use chrono::prelude::*;

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

use chrono::{Utc, TimeZone};

/// Datastore Management
///
/// A Datastore can store severals backups, and provides the
/// management interface for backup.
pub struct DataStore {
    chunk_store: Arc<ChunkStore>,
    gc_mutex: Mutex<bool>,
}

/// Uniquely identify backups (relative to data store)
#[derive(Debug)]
pub struct BackupDir {
    /// Type of backup
    pub backup_type: String,
    /// Unique (for this type) ID
    pub backup_id: String,
    /// Backup timestamp
    pub backup_time: DateTime<Utc>,
}

impl BackupDir {

    pub fn relative_path(&self) ->  PathBuf  {

        let mut relative_path = PathBuf::new();

        relative_path.push(&self.backup_type);

        relative_path.push(&self.backup_id);

        let date_str = self.backup_time.format("%Y-%m-%dT%H:%M:%S").to_string();

        relative_path.push(&date_str);

        relative_path
    }
}

/// Detailed Backup Information
#[derive(Debug)]
pub struct BackupInfo {
    /// the backup directory
    pub backup_dir: BackupDir,
    /// List of data files
    pub files: Vec<String>,
}


lazy_static!{
    static ref datastore_map: Mutex<HashMap<String, Arc<DataStore>>> =  Mutex::new(HashMap::new());
}

impl DataStore {

    pub fn lookup_datastore(name: &str) -> Result<Arc<DataStore>, Error> {

        let config = datastore::config()?;
        let (_, store_config) = config.sections.get(name)
            .ok_or(format_err!("no such datastore '{}'", name))?;

        let path = store_config["path"].as_str().unwrap();

        let mut map = datastore_map.lock().unwrap();

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

        Ok(Self {
            chunk_store: Arc::new(chunk_store),
            gc_mutex: Mutex::new(false),
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

    pub fn create_backup_dir(
        &self,
        backup_type: &str,
        backup_id: &str,
        backup_time: DateTime<Utc>,
    ) ->  Result<(PathBuf, bool), io::Error> {
        let mut relative_path = PathBuf::new();

        relative_path.push(backup_type);

        relative_path.push(backup_id);

        // create intermediate path first:
        let mut full_path = self.base_path();
        full_path.push(&relative_path);
        std::fs::create_dir_all(&full_path)?;

        let date_str = backup_time.format("%Y-%m-%dT%H:%M:%S").to_string();

        println!("date: {}", date_str);
        relative_path.push(&date_str);
        full_path.push(&date_str);

        // create the last component now
        match std::fs::create_dir(&full_path) {
            Ok(_) => Ok((relative_path, true)),
            Err(ref e) if e.kind() == io::ErrorKind::AlreadyExists => Ok((relative_path, false)),
            Err(e) => Err(e)
        }
    }

    pub fn list_backups(&self) -> Result<Vec<BackupInfo>, Error> {
        let path = self.base_path();

        let mut list = vec![];

        lazy_static! {
            static ref BACKUP_FILE_REGEX: regex::Regex = regex::Regex::new(r"^.*\.([fd]idx)$").unwrap();
            static ref BACKUP_TYPE_REGEX: regex::Regex = regex::Regex::new(r"^(host|vm|ct)$").unwrap();
            static ref BACKUP_ID_REGEX: regex::Regex = regex::Regex::new(r"^[A-Za-z][A-Za-z0-9_-]+$").unwrap();
            static ref BACKUP_DATE_REGEX: regex::Regex = regex::Regex::new(
                r"^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}$").unwrap();
        }

        tools::scandir(libc::AT_FDCWD, &path, &BACKUP_TYPE_REGEX, |l0_fd, backup_type, file_type| {
            if file_type != nix::dir::Type::Directory { return Ok(()); }
            tools::scandir(l0_fd, backup_type, &BACKUP_ID_REGEX, |l1_fd, backup_id, file_type| {
                if file_type != nix::dir::Type::Directory { return Ok(()); }
                tools::scandir(l1_fd, backup_id, &BACKUP_DATE_REGEX, |l2_fd, backup_time, file_type| {
                    if file_type != nix::dir::Type::Directory { return Ok(()); }

                    let dt = Utc.datetime_from_str(backup_time, "%Y-%m-%dT%H:%M:%S")?;

                    let mut files = vec![];

                    tools::scandir(l2_fd, backup_time, &BACKUP_FILE_REGEX, |_, filename, file_type| {
                        if file_type != nix::dir::Type::File { return Ok(()); }
                        files.push(filename.to_owned());
                        Ok(())
                    })?;

                    list.push(BackupInfo {
                        backup_dir: BackupDir {
                            backup_type: backup_type.to_owned(),
                            backup_id: backup_id.to_owned(),
                            backup_time: dt,
                        },
                        files,
                    });

                    Ok(())
                })
            })
        })?;

        Ok(list)
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

    pub fn garbage_collection(&self) -> Result<(), Error> {

        if let Ok(ref mut _mutex) = self.gc_mutex.try_lock() {

            let mut gc_status = GarbageCollectionStatus::default();
            gc_status.used_bytes = 0;

            println!("Start GC phase1 (mark chunks)");

            self.mark_used_chunks(&mut gc_status)?;

            println!("Start GC phase2 (sweep unused chunks)");
            self.chunk_store.sweep_unused_chunks(&mut gc_status)?;

            println!("Used bytes: {}", gc_status.used_bytes);
            println!("Used chunks: {}", gc_status.used_chunks);
            println!("Disk bytes: {}", gc_status.disk_bytes);
            println!("Disk chunks: {}", gc_status.disk_chunks);

        } else {
            println!("Start GC failed - (already running/locked)");
        }

        Ok(())
    }
}
