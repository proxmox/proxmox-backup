use failure::*;

use std::path::{PathBuf, Path};
use std::sync::Mutex;

use crate::config::datastore;
use super::chunk_store::*;
use super::image_index::*;

pub struct DataStore {
    chunk_store: ChunkStore,
    gc_mutex: Mutex<bool>,
}

impl DataStore {

    pub fn open(store_name: &str) -> Result<Self, Error> {

        let config = datastore::config()?;
        let (_, store_config) = config.sections.get(store_name)
            .ok_or(format_err!("no such datastore '{}'", store_name))?;

        let path = store_config["path"].as_str().unwrap();

        let chunk_store = ChunkStore::open(store_name, path)?;

        Ok(Self {
            chunk_store: chunk_store,
            gc_mutex: Mutex::new(false),
        })
    }

    pub fn create_image_writer<P: AsRef<Path>>(&mut self, filename: P, size: usize, chunk_size: usize) -> Result<ImageIndexWriter, Error> {

        let index = ImageIndexWriter::create(&mut self.chunk_store, filename.as_ref(), size, chunk_size)?;

        Ok(index)
    }

    pub fn open_image_reader<P: AsRef<Path>>(&self, filename: P) -> Result<ImageIndexReader, Error> {

        let index = ImageIndexReader::open(&self.chunk_store, filename.as_ref())?;

        Ok(index)
    }

    pub fn list_images(&self) -> Result<Vec<PathBuf>, Error> {
        let base = self.chunk_store.base_path();

        let mut list = vec![];

        for entry in std::fs::read_dir(base)? {
            let entry = entry?;
            if entry.file_type()?.is_file() {
                let path = entry.path();
                if let Some(ext) = path.extension() {
                    if ext == "iidx" {
                        list.push(path);
                    }
                }
            }
        }

        Ok(list)
    }

    fn mark_used_chunks(&self, status: &mut GarbageCollectionStatus) -> Result<(), Error> {

        let image_list = self.list_images()?;

        for path in image_list {
            let index = self.open_image_reader(path)?;
            index.mark_used_chunks(status)?;
        }

        Ok(())
   }

    pub fn garbage_collection(&self) -> Result<(), Error> {

        if let Ok(ref mut mutex) = self.gc_mutex.try_lock() {

            let mut gc_status = GarbageCollectionStatus::default();
            gc_status.used_bytes = 0;

            println!("Start GC phase1 (mark chunks)");

            self.mark_used_chunks(&mut gc_status)?;

            println!("Start GC phase2 (sweep unused chunks)");
            self.chunk_store.sweep_used_chunks(&mut gc_status)?;

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
