use failure::*;

use std::path::{PathBuf, Path};

use crate::config::datastore;
use super::chunk_store::*;
use super::image_index::*;

pub struct DataStore {
    chunk_store: ChunkStore,
}

impl DataStore {

    pub fn open(store_name: &str) -> Result<Self, Error> {

        let config = datastore::config()?;
        let (_, store_config) = config.sections.get(store_name)
            .ok_or(format_err!("no such datastore '{}'", store_name))?;

        let path = store_config["path"].as_str().unwrap();

        let chunk_store = ChunkStore::open(path)?;

        Ok(Self {
            chunk_store: chunk_store,
        })
    }

    pub fn create_image_writer<P: AsRef<Path>>(&mut self, filename: P, size: usize) -> Result<ImageIndexWriter, Error> {

        let index = ImageIndexWriter::create(&mut self.chunk_store, filename.as_ref(), size)?;

        Ok(index)
    }

    pub fn open_image_reader<P: AsRef<Path>>(&mut self, filename: P) -> Result<ImageIndexReader, Error> {

        let index = ImageIndexReader::open(&mut self.chunk_store, filename.as_ref())?;

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
                    if ext == "idx" {
                        list.push(path);
                    }
                }
            }
        }

        Ok(list)
    }

    fn mark_used_chunks(&mut self) -> Result<(), Error> {

        let image_list = self.list_images()?;

        for path in image_list {
            let mut index = self.open_image_reader(path)?;
            index.mark_used_chunks()?;
        }

        Ok(())
   }

    pub fn garbage_collection(&mut self) -> Result<(), Error> {

        self.mark_used_chunks()?;

        self.chunk_store.sweep_used_chunks()?;

        Ok(())
    }
}
