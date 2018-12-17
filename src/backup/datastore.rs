use failure::*;

use std::path::Path;

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
}
