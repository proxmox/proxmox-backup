use std::io::{self, Seek, SeekFrom};
use std::ops::Range;
use std::sync::{Arc, Mutex};
use std::task::Context;
use std::pin::Pin;

use anyhow::{bail, format_err, Error};

use pxar::accessor::{MaybeReady, ReadAt, ReadAtOperation};

use pbs_datastore::dynamic_index::DynamicIndexReader;
use pbs_datastore::read_chunk::ReadChunk;
use pbs_datastore::index::IndexFile;
use pbs_tools::lru_cache::LruCache;

struct CachedChunk {
    range: Range<u64>,
    data: Vec<u8>,
}

impl CachedChunk {
    /// Perform sanity checks on the range and data size:
    pub fn new(range: Range<u64>, data: Vec<u8>) -> Result<Self, Error> {
        if data.len() as u64 != range.end - range.start {
            bail!(
                "read chunk with wrong size ({} != {})",
                data.len(),
                range.end - range.start,
            );
        }
        Ok(Self { range, data })
    }
}

pub struct BufferedDynamicReader<S> {
    store: S,
    index: DynamicIndexReader,
    archive_size: u64,
    read_buffer: Vec<u8>,
    buffered_chunk_idx: usize,
    buffered_chunk_start: u64,
    read_offset: u64,
    lru_cache: LruCache<usize, CachedChunk>,
}

struct ChunkCacher<'a, S> {
    store: &'a mut S,
    index: &'a DynamicIndexReader,
}

impl<'a, S: ReadChunk> pbs_tools::lru_cache::Cacher<usize, CachedChunk> for ChunkCacher<'a, S> {
    fn fetch(&mut self, index: usize) -> Result<Option<CachedChunk>, Error> {
        let info = match self.index.chunk_info(index) {
            Some(info) => info,
            None => bail!("chunk index out of range"),
        };
        let range = info.range;
        let data = self.store.read_chunk(&info.digest)?;
        CachedChunk::new(range, data).map(Some)
    }
}

impl<S: ReadChunk> BufferedDynamicReader<S> {
    pub fn new(index: DynamicIndexReader, store: S) -> Self {
        let archive_size = index.index_bytes();
        Self {
            store,
            index,
            archive_size,
            read_buffer: Vec::with_capacity(1024 * 1024),
            buffered_chunk_idx: 0,
            buffered_chunk_start: 0,
            read_offset: 0,
            lru_cache: LruCache::new(32),
        }
    }

    pub fn archive_size(&self) -> u64 {
        self.archive_size
    }

    fn buffer_chunk(&mut self, idx: usize) -> Result<(), Error> {
        //let (start, end, data) = self.lru_cache.access(
        let cached_chunk = self.lru_cache.access(
            idx,
            &mut ChunkCacher {
                store: &mut self.store,
                index: &self.index,
            },
        )?.ok_or_else(|| format_err!("chunk not found by cacher"))?;

        // fixme: avoid copy
        self.read_buffer.clear();
        self.read_buffer.extend_from_slice(&cached_chunk.data);

        self.buffered_chunk_idx = idx;

        self.buffered_chunk_start = cached_chunk.range.start;
        //println!("BUFFER {} {}",  self.buffered_chunk_start, end);
        Ok(())
    }
}

impl<S: ReadChunk> crate::tools::BufferedRead for BufferedDynamicReader<S> {
    fn buffered_read(&mut self, offset: u64) -> Result<&[u8], Error> {
        if offset == self.archive_size {
            return Ok(&self.read_buffer[0..0]);
        }

        let buffer_len = self.read_buffer.len();
        let index = &self.index;

        // optimization for sequential read
        if buffer_len > 0
            && ((self.buffered_chunk_idx + 1) < index.index().len())
            && (offset >= (self.buffered_chunk_start + (self.read_buffer.len() as u64)))
        {
            let next_idx = self.buffered_chunk_idx + 1;
            let next_end = index.chunk_end(next_idx);
            if offset < next_end {
                self.buffer_chunk(next_idx)?;
                let buffer_offset = (offset - self.buffered_chunk_start) as usize;
                return Ok(&self.read_buffer[buffer_offset..]);
            }
        }

        if (buffer_len == 0)
            || (offset < self.buffered_chunk_start)
            || (offset >= (self.buffered_chunk_start + (self.read_buffer.len() as u64)))
        {
            let end_idx = index.index().len() - 1;
            let end = index.chunk_end(end_idx);
            let idx = index.binary_search(0, 0, end_idx, end, offset)?;
            self.buffer_chunk(idx)?;
        }

        let buffer_offset = (offset - self.buffered_chunk_start) as usize;
        Ok(&self.read_buffer[buffer_offset..])
    }
}

impl<S: ReadChunk> std::io::Read for BufferedDynamicReader<S> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, std::io::Error> {
        use crate::tools::BufferedRead;
        use std::io::{Error, ErrorKind};

        let data = match self.buffered_read(self.read_offset) {
            Ok(v) => v,
            Err(err) => return Err(Error::new(ErrorKind::Other, err.to_string())),
        };

        let n = if data.len() > buf.len() {
            buf.len()
        } else {
            data.len()
        };

        buf[0..n].copy_from_slice(&data[0..n]);

        self.read_offset += n as u64;

        Ok(n)
    }
}

impl<S: ReadChunk> std::io::Seek for BufferedDynamicReader<S> {
    fn seek(&mut self, pos: SeekFrom) -> Result<u64, std::io::Error> {
        let new_offset = match pos {
            SeekFrom::Start(start_offset) => start_offset as i64,
            SeekFrom::End(end_offset) => (self.archive_size as i64) + end_offset,
            SeekFrom::Current(offset) => (self.read_offset as i64) + offset,
        };

        use std::io::{Error, ErrorKind};
        if (new_offset < 0) || (new_offset > (self.archive_size as i64)) {
            return Err(Error::new(
                ErrorKind::Other,
                format!(
                    "seek is out of range {} ([0..{}])",
                    new_offset, self.archive_size
                ),
            ));
        }
        self.read_offset = new_offset as u64;

        Ok(self.read_offset)
    }
}

/// This is a workaround until we have cleaned up the chunk/reader/... infrastructure for better
/// async use!
///
/// Ideally BufferedDynamicReader gets replaced so the LruCache maps to `BroadcastFuture<Chunk>`,
/// so that we can properly access it from multiple threads simultaneously while not issuing
/// duplicate simultaneous reads over http.
#[derive(Clone)]
pub struct LocalDynamicReadAt<R: ReadChunk> {
    inner: Arc<Mutex<BufferedDynamicReader<R>>>,
}

impl<R: ReadChunk> LocalDynamicReadAt<R> {
    pub fn new(inner: BufferedDynamicReader<R>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(inner)),
        }
    }
}

impl<R: ReadChunk> ReadAt for LocalDynamicReadAt<R> {
    fn start_read_at<'a>(
        self: Pin<&'a Self>,
        _cx: &mut Context,
        buf: &'a mut [u8],
        offset: u64,
    ) -> MaybeReady<io::Result<usize>, ReadAtOperation<'a>> {
        use std::io::Read;
        MaybeReady::Ready(tokio::task::block_in_place(move || {
            let mut reader = self.inner.lock().unwrap();
            reader.seek(SeekFrom::Start(offset))?;
            Ok(reader.read(buf)?)
        }))
    }

    fn poll_complete<'a>(
        self: Pin<&'a Self>,
        _op: ReadAtOperation<'a>,
    ) -> MaybeReady<io::Result<usize>, ReadAtOperation<'a>> {
        panic!("LocalDynamicReadAt::start_read_at returned Pending");
    }
}
