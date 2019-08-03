use failure::*;
use std::sync::Arc;

use super::datastore::*;
use super::crypt_config::*;
use super::data_chunk::*;

/// The ReadChunk trait allows reading backup data chunks (local or remote)
pub trait ReadChunk {
    /// Returns the decoded chunk data
    fn read_chunk(&mut self, digest:&[u8; 32]) -> Result<Vec<u8>, Error>;
}

pub struct LocalChunkReader {
    store: Arc<DataStore>,
    crypt_config: Option<Arc<CryptConfig>>,
}

impl LocalChunkReader {

    pub fn new(store: Arc<DataStore>, crypt_config: Option<Arc<CryptConfig>>) -> Self {
        Self { store, crypt_config }
    }
}

impl ReadChunk for LocalChunkReader {

    fn read_chunk(&mut self, digest:&[u8; 32]) -> Result<Vec<u8>, Error> {

        let digest_str = proxmox::tools::digest_to_hex(digest);
        println!("READ CHUNK {}", digest_str);

        let (path, _) = self.store.chunk_path(digest);
        let raw_data = proxmox::tools::fs::file_get_contents(&path)?;
        let chunk = DataChunk::from_raw(raw_data, *digest)?;
        chunk.verify_crc()?;

        let raw_data = match self.crypt_config {
            Some(ref crypt_config) => chunk.decode(Some(crypt_config))?,
            None => chunk.decode(None)?,
        };

        Ok(raw_data)
    }
}
