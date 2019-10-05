use std::collections::HashMap;
use std::sync::Arc;

use failure::*;

use super::BackupReader;
use crate::backup::{ReadChunk, DataChunk, CryptConfig};

/// Read chunks from remote host using ``BackupReader``
pub struct RemoteChunkReader {
    client: Arc<BackupReader>,
    crypt_config: Option<Arc<CryptConfig>>,
    cache_hint: HashMap<[u8; 32], usize>,
    cache:  HashMap<[u8; 32], Vec<u8>>,
}

impl RemoteChunkReader {

    /// Create a new instance.
    ///
    /// Chunks listed in ``cache_hint`` are cached and kept in RAM.
    pub fn new(
        client: Arc<BackupReader>,
        crypt_config: Option<Arc<CryptConfig>>,
        cache_hint: HashMap<[u8; 32], usize>,
    ) -> Self {

        Self { client, crypt_config, cache_hint, cache: HashMap::new() }
    }
}

impl ReadChunk for RemoteChunkReader {

    fn read_chunk(&mut self, digest:&[u8; 32]) -> Result<Vec<u8>, Error> {

        let mut chunk_data = Vec::with_capacity(4*1024*1024);

        if let Some(raw_data) = self.cache.get(digest) {
            return Ok(raw_data.to_vec());
        }

        let use_cache = self.cache_hint.contains_key(digest);

        futures::executor::block_on(self.client.download_chunk(&digest, &mut chunk_data))?;

        let chunk = DataChunk::from_raw(chunk_data, *digest)?;
        chunk.verify_crc()?;

        let raw_data = match self.crypt_config {
            Some(ref crypt_config) => chunk.decode(Some(crypt_config))?,
            None => chunk.decode(None)?,
        };

        if use_cache {
            self.cache.insert(*digest, raw_data.to_vec());
        }

        Ok(raw_data)
    }
}
