use std::collections::HashMap;
use std::sync::Arc;

use failure::*;

use super::BackupReader;
use crate::backup::{ReadChunk, DataBlob, CryptConfig};

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

    fn read_raw_chunk(&mut self, digest:&[u8; 32]) -> Result<DataBlob, Error> {

        let mut chunk_data = Vec::with_capacity(4*1024*1024);

        tokio::task::block_in_place(|| futures::executor::block_on(self.client.download_chunk(&digest, &mut chunk_data)))?;

        let chunk = DataBlob::from_raw(chunk_data)?;
        chunk.verify_crc()?;

        Ok(chunk)
    }

    fn read_chunk(&mut self, digest:&[u8; 32]) -> Result<Vec<u8>, Error> {

        if let Some(raw_data) = self.cache.get(digest) {
            return Ok(raw_data.to_vec());
        }

        let chunk = self.read_raw_chunk(digest)?;

        let raw_data =  chunk.decode(self.crypt_config.as_ref().map(Arc::as_ref))?;

        // fixme: verify digest?

        let use_cache = self.cache_hint.contains_key(digest);
        if use_cache {
            self.cache.insert(*digest, raw_data.to_vec());
        }

        Ok(raw_data)
    }

}
