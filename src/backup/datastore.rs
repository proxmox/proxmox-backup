use failure::*;

use chrono::prelude::*;

use std::path::{PathBuf, Path};
use std::collections::HashMap;
use lazy_static::lazy_static;
use std::sync::{Mutex, Arc};

use crate::tools;
use crate::config::datastore;
use super::chunk_store::*;
use super::image_index::*;
use super::archive_index::*;

use chrono::{Utc, TimeZone};

pub struct DataStore {
    chunk_store: Arc<ChunkStore>,
    gc_mutex: Mutex<bool>,
}

#[derive(Debug)]
pub struct BackupInfo {
    pub backup_type: String,
    pub backup_id: String,
    pub backup_time: DateTime<Utc>,
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

        if let Ok(datastore) = DataStore::open(name)  {
            let datastore = Arc::new(datastore);
            map.insert(name.to_string(), datastore.clone());
            return Ok(datastore);
        }

        bail!("store not found");
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

    pub fn create_image_writer<P: AsRef<Path>>(&self, filename: P, size: usize, chunk_size: usize) -> Result<ImageIndexWriter, Error> {

        let index = ImageIndexWriter::create(self.chunk_store.clone(), filename.as_ref(), size, chunk_size)?;

        Ok(index)
    }

    pub fn open_image_reader<P: AsRef<Path>>(&self, filename: P) -> Result<ImageIndexReader, Error> {

        let index = ImageIndexReader::open(self.chunk_store.clone(), filename.as_ref())?;

        Ok(index)
    }

    pub fn create_archive_writer<P: AsRef<Path>>(
        &self, filename: P,
        chunk_size: usize
    ) -> Result<ArchiveIndexWriter, Error> {

        let index = ArchiveIndexWriter::create(
            self.chunk_store.clone(), filename.as_ref(), chunk_size)?;

        Ok(index)
    }

    pub fn open_archive_reader<P: AsRef<Path>>(&self, filename: P) -> Result<ArchiveIndexReader, Error> {

        let index = ArchiveIndexReader::open(self.chunk_store.clone(), filename.as_ref())?;

        Ok(index)
    }

    pub fn base_path(&self) -> PathBuf {
        self.chunk_store.base_path()
    }

    pub fn get_backup_dir(
        &self,
        backup_type: &str,
        backup_id: &str,
        backup_time: DateTime<Utc>,
    ) ->  PathBuf  {

        let mut relative_path = PathBuf::new();

        relative_path.push(backup_type);

        relative_path.push(backup_id);

        let date_str = backup_time.format("%Y-%m-%dT%H:%M:%S").to_string();

        relative_path.push(&date_str);

        relative_path
    }

    pub fn create_backup_dir(
        &self,
        backup_type: &str,
        backup_id: &str,
        backup_time: i64,
    ) ->  Result<PathBuf, Error> {
        let mut relative_path = PathBuf::new();

        relative_path.push(backup_type);

        relative_path.push(backup_id);

        let dt = Utc.timestamp(backup_time, 0);
        let date_str = dt.format("%Y-%m-%dT%H:%M:%S").to_string();

        println!("date: {}", date_str);

        relative_path.push(&date_str);


        let mut full_path = self.base_path();
        full_path.push(&relative_path);

        std::fs::create_dir_all(&full_path)?;

        Ok(relative_path)
    }

    pub fn list_backups(&self) -> Result<Vec<BackupInfo>, Error> {
        let path = self.base_path();

        let mut list = vec![];

        lazy_static! {
            static ref BACKUP_TYPE_REGEX: regex::Regex = regex::Regex::new(r"^(host|vm|ct)$").unwrap();
            static ref BACKUP_ID_REGEX: regex::Regex = regex::Regex::new(r"^[A-Za-z][A-Za-z0-9_-]+$").unwrap();
            static ref BACKUP_DATE_REGEX: regex::Regex = regex::Regex::new(
                r"^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}$").unwrap();
        }

        tools::scandir(libc::AT_FDCWD, &path, &BACKUP_TYPE_REGEX, |l0_fd, backup_type, file_type| {
            if file_type != nix::dir::Type::Directory { return Ok(()); }
            tools::scandir(l0_fd, backup_type, &BACKUP_ID_REGEX, |l1_fd, backup_id, file_type| {
                if file_type != nix::dir::Type::Directory { return Ok(()); }
                tools::scandir(l1_fd, backup_id, &BACKUP_DATE_REGEX, |_, backup_time, file_type| {
                    if file_type != nix::dir::Type::Directory { return Ok(()); }

                    let dt = Utc.datetime_from_str(backup_time, "%Y-%m-%dT%H:%M:%S")?;

                    list.push(BackupInfo {
                        backup_type: backup_type.to_owned(),
                        backup_id: backup_id.to_owned(),
                        backup_time: dt,
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
                if ext == "iidx" {
                    list.push(path);
                } else if ext == "aidx" {
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
                if ext == "iidx" {
                    let index = self.open_image_reader(&path)?;
                    index.mark_used_chunks(status)?;
                } else if ext == "aidx" {
                    let index = self.open_archive_reader(&path)?;
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
