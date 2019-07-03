use failure::*;
use futures::future::Future;
use std::sync::Arc;

use super::BackupReader;
use crate::backup::{ReadChunk, DataChunk, CryptConfig};

/// Read chunks from remote host using ``BackupReader``
pub struct RemoteChunkReader {
    client: Arc<BackupReader>,
    crypt_config: Option<Arc<CryptConfig>>,
}

impl RemoteChunkReader {

    pub fn new(client: Arc<BackupReader>, crypt_config: Option<Arc<CryptConfig>>) -> Self {
        Self { client, crypt_config }
    }
}

impl ReadChunk for RemoteChunkReader {

    fn read_chunk(&mut self, digest:&[u8; 32]) -> Result<Vec<u8>, Error> {

        let digest_str = proxmox::tools::digest_to_hex(digest);

        let writer = Vec::with_capacity(4*1024*1024);

        let chunk_data = self.client.download_chunk(&digest, writer).wait()?;

        let chunk = DataChunk::from_raw(chunk_data, *digest)?;
        chunk.verify_crc()?;

        let raw_data = match self.crypt_config {
            Some(ref crypt_config) => chunk.decode(Some(crypt_config))?,
            None => chunk.decode(None)?,
        };

        Ok(raw_data)
    }
}
