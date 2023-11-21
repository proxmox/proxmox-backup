use std::future::Future;
use std::pin::Pin;

use anyhow::Error;

use crate::data_blob::DataBlob;

/// The ReadChunk trait allows reading backup data chunks (local or remote)
pub trait ReadChunk {
    /// Returns the encoded chunk data
    fn read_raw_chunk(&self, digest: &[u8; 32]) -> Result<DataBlob, Error>;

    /// Returns the decoded chunk data
    fn read_chunk(&self, digest: &[u8; 32]) -> Result<Vec<u8>, Error>;
}

pub trait AsyncReadChunk: Send + Sync {
    /// Returns the encoded chunk data
    fn read_raw_chunk<'a>(
        &'a self,
        digest: &'a [u8; 32],
    ) -> Pin<Box<dyn Future<Output = Result<DataBlob, Error>> + Send + 'a>>;

    /// Returns the decoded chunk data
    fn read_chunk<'a>(
        &'a self,
        digest: &'a [u8; 32],
    ) -> Pin<Box<dyn Future<Output = Result<Vec<u8>, Error>> + Send + 'a>>;
}
