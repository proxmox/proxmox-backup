//! An async and concurrency safe data reader backed by a local LRU cache.

use std::future::Future;
use std::io::SeekFrom;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use anyhow::Error;
use futures::ready;
use tokio::io::{AsyncRead, AsyncSeek, ReadBuf};

use proxmox_lang::error::io_err_other;
use proxmox_lang::io_format_err;

use pbs_tools::async_lru_cache::{AsyncCacher, AsyncLruCache};

use crate::index::IndexFile;
use crate::read_chunk::AsyncReadChunk;

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

impl<I: IndexFile + Send + Sync + 'static, R: AsyncReadChunk + Send + Sync + 'static>
    CachedChunkReader<I, R>
{
    /// Returns a SeekableCachedChunkReader based on this instance, which implements AsyncSeek and
    /// AsyncRead for use in interfaces which require that. Direct use of read_at is preferred
    /// otherwise.
    pub fn seekable(self) -> SeekableCachedChunkReader<I, R> {
        SeekableCachedChunkReader {
            index_bytes: self.index.index_bytes(),
            reader: Arc::new(self),
            position: 0,
            read_future: None,
        }
    }
}

pub struct SeekableCachedChunkReader<
    I: IndexFile + Send + Sync + 'static,
    R: AsyncReadChunk + Send + Sync + 'static,
> {
    reader: Arc<CachedChunkReader<I, R>>,
    index_bytes: u64,
    position: u64,
    #[allow(clippy::type_complexity)]
    read_future: Option<Pin<Box<dyn Future<Output = Result<(Vec<u8>, usize), Error>> + Send>>>,
}

impl<I, R> AsyncSeek for SeekableCachedChunkReader<I, R>
where
    I: IndexFile + Send + Sync + 'static,
    R: AsyncReadChunk + Send + Sync + 'static,
{
    fn start_seek(self: Pin<&mut Self>, pos: SeekFrom) -> tokio::io::Result<()> {
        let this = Pin::get_mut(self);
        let seek_to_pos = match pos {
            SeekFrom::Start(offset) => offset as i64,
            SeekFrom::End(offset) => this.index_bytes as i64 + offset,
            SeekFrom::Current(offset) => this.position as i64 + offset,
        };
        if seek_to_pos < 0 {
            return Err(io_format_err!("cannot seek to negative values"));
        } else if seek_to_pos > this.index_bytes as i64 {
            this.position = this.index_bytes;
        } else {
            this.position = seek_to_pos as u64;
        }
        Ok(())
    }

    fn poll_complete(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<tokio::io::Result<u64>> {
        Poll::Ready(Ok(self.position))
    }
}

impl<I, R> AsyncRead for SeekableCachedChunkReader<I, R>
where
    I: IndexFile + Send + Sync + 'static,
    R: AsyncReadChunk + Send + Sync + 'static,
{
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context,
        buf: &mut ReadBuf,
    ) -> Poll<tokio::io::Result<()>> {
        let this = Pin::get_mut(self);

        let offset = this.position;
        let wanted = buf.capacity();
        let reader = Arc::clone(&this.reader);

        let fut = this.read_future.get_or_insert_with(|| {
            Box::pin(async move {
                let mut read_buf = vec![0u8; wanted];
                let read = reader.read_at(&mut read_buf[..wanted], offset).await?;
                Ok((read_buf, read))
            })
        });

        let ret = match ready!(fut.as_mut().poll(cx)) {
            Ok((read_buf, read)) => {
                buf.put_slice(&read_buf[..read]);
                this.position += read as u64;
                Ok(())
            }
            Err(err) => Err(io_err_other(err)),
        };

        // future completed, drop
        this.read_future = None;

        Poll::Ready(ret)
    }
}
