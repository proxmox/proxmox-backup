//! An async and concurrency safe data reader backed by a local LRU cache.

use anyhow::Error;

use std::future::Future;
use std::sync::Arc;

use crate::backup::{AsyncReadChunk, IndexFile};
use crate::tools::async_lru_cache::{AsyncCacher, AsyncLruCache};

struct AsyncChunkCacher<T> {
    reader: Arc<T>,
}

impl<T: AsyncReadChunk + Send + Sync + 'static> AsyncCacher<[u8; 32], Arc<Vec<u8>>>
    for AsyncChunkCacher<T>
{
    fn fetch(
        &self,
        key: [u8; 32],
    ) -> Box<dyn Future<Output = Result<Option<Arc<Vec<u8>>>, Error>> + Send> {
        let reader = Arc::clone(&self.reader);
        Box::new(async move {
            AsyncReadChunk::read_chunk(reader.as_ref(), &key)
                .await
                .map(|x| Some(Arc::new(x)))
        })
    }
}

/// Allows arbitrary data reads from an Index via an AsyncReadChunk implementation, using an LRU
/// cache internally to cache chunks and provide support for multiple concurrent reads (potentially
/// to the same chunk).
pub struct CachedChunkReader<I: IndexFile, R: AsyncReadChunk + Send + Sync + 'static> {
    cache: Arc<AsyncLruCache<[u8; 32], Arc<Vec<u8>>>>,
    cacher: AsyncChunkCacher<R>,
    index: I,
}

impl<I: IndexFile, R: AsyncReadChunk + Send + Sync + 'static> CachedChunkReader<I, R> {
    /// Create a new reader with a local LRU cache containing 'capacity' chunks.
    pub fn new(reader: R, index: I, capacity: usize) -> Self {
        let cache = Arc::new(AsyncLruCache::new(capacity));
        Self::new_with_cache(reader, index, cache)
    }

    /// Create a new reader with a custom LRU cache. Use this to share a cache between multiple
    /// readers.
    pub fn new_with_cache(
        reader: R,
        index: I,
        cache: Arc<AsyncLruCache<[u8; 32], Arc<Vec<u8>>>>,
    ) -> Self {
        Self {
            cache,
            cacher: AsyncChunkCacher {
                reader: Arc::new(reader),
            },
            index,
        }
    }

    /// Read data at a given byte offset into a variable size buffer. Returns the amount of bytes
    /// read, which will always be the size of the buffer except when reaching EOF.
    pub async fn read_at(&self, buf: &mut [u8], offset: u64) -> Result<usize, Error> {
        let size = buf.len();
        let mut read: usize = 0;
        while read < size {
            let cur_offset = offset + read as u64;
            if let Some(chunk) = self.index.chunk_from_offset(cur_offset) {
                // chunk indices retrieved from chunk_from_offset always resolve to Some(_)
                let info = self.index.chunk_info(chunk.0).unwrap();

                // will never be None, see AsyncChunkCacher
                let data = self.cache.access(info.digest, &self.cacher).await?.unwrap();

                let want_bytes = ((info.range.end - cur_offset) as usize).min(size - read);
                let slice = &mut buf[read..(read + want_bytes)];
                let intra_chunk = chunk.1 as usize;
                slice.copy_from_slice(&data[intra_chunk..(intra_chunk + want_bytes)]);
                read += want_bytes;
            } else {
                // EOF
                break;
            }
        }
        Ok(read)
    }
}
