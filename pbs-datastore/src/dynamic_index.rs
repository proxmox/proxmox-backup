use std::fs::File;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::ops::Range;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::Context;

use anyhow::{bail, format_err, Error};

use proxmox_io::ReadExt;
use proxmox_sys::mmap::Mmap;
use proxmox_sys::process_locker::ProcessLockSharedGuard;
use proxmox_uuid::Uuid;
use pxar::accessor::{MaybeReady, ReadAt, ReadAtOperation};

use pbs_tools::lru_cache::LruCache;

use crate::chunk_stat::ChunkStat;
use crate::chunk_store::ChunkStore;
use crate::data_blob::{DataBlob, DataChunkBuilder};
use crate::file_formats;
use crate::index::{ChunkReadInfo, IndexFile};
use crate::read_chunk::ReadChunk;
use crate::Chunker;

/// Header format definition for dynamic index files (`.dixd`)
#[repr(C)]
pub struct DynamicIndexHeader {
    pub magic: [u8; 8],
    pub uuid: [u8; 16],
    pub ctime: i64,
    /// Sha256 over the index ``SHA256(offset1||digest1||offset2||digest2||...)``
    pub index_csum: [u8; 32],
    reserved: [u8; 4032], // overall size is one page (4096 bytes)
}
proxmox_lang::static_assert_size!(DynamicIndexHeader, 4096);
// TODO: Once non-Copy unions are stabilized, use:
// union DynamicIndexHeader {
//     reserved: [u8; 4096],
//     pub data: DynamicIndexHeaderData,
// }

impl DynamicIndexHeader {
    /// Convenience method to allocate a zero-initialized header struct.
    pub fn zeroed() -> Box<Self> {
        unsafe {
            Box::from_raw(std::alloc::alloc_zeroed(std::alloc::Layout::new::<Self>()) as *mut Self)
        }
    }

    pub fn as_bytes(&self) -> &[u8] {
        unsafe {
            std::slice::from_raw_parts(
                self as *const Self as *const u8,
                std::mem::size_of::<Self>(),
            )
        }
    }
}

#[derive(Clone, Debug)]
#[repr(C)]
pub struct DynamicEntry {
    end_le: u64,
    digest: [u8; 32],
}

impl DynamicEntry {
    #[inline]
    pub fn end(&self) -> u64 {
        u64::from_le(self.end_le)
    }
}

pub struct DynamicIndexReader {
    _file: File,
    pub size: usize,
    index: Mmap<DynamicEntry>,
    pub uuid: [u8; 16],
    pub ctime: i64,
    pub index_csum: [u8; 32],
}

impl DynamicIndexReader {
    pub fn open(path: &Path) -> Result<Self, Error> {
        File::open(path)
            .map_err(Error::from)
            .and_then(Self::new)
            .map_err(|err| format_err!("Unable to open dynamic index {:?} - {}", path, err))
    }

    pub fn index(&self) -> &[DynamicEntry] {
        &self.index
    }

    pub fn new(mut file: std::fs::File) -> Result<Self, Error> {
        // FIXME: This is NOT OUR job! Check the callers of this method and remove this!
        file.seek(SeekFrom::Start(0))?;

        let header_size = std::mem::size_of::<DynamicIndexHeader>();

        let rawfd = file.as_raw_fd();
        let stat = match nix::sys::stat::fstat(rawfd) {
            Ok(stat) => stat,
            Err(err) => bail!("fstat failed - {}", err),
        };

        let size = stat.st_size as usize;

        if size < header_size {
            bail!("index too small ({})", stat.st_size);
        }

        let header: Box<DynamicIndexHeader> = unsafe { file.read_host_value_boxed()? };

        if header.magic != file_formats::DYNAMIC_SIZED_CHUNK_INDEX_1_0 {
            bail!("got unknown magic number");
        }

        let ctime = proxmox_time::epoch_i64();

        let index_size = stat.st_size as usize - header_size;
        let index_count = index_size / 40;
        if index_count * 40 != index_size {
            bail!("got unexpected file size");
        }

        let index = unsafe {
            Mmap::map_fd(
                rawfd,
                header_size as u64,
                index_count,
                nix::sys::mman::ProtFlags::PROT_READ,
                nix::sys::mman::MapFlags::MAP_PRIVATE,
            )?
        };

        Ok(Self {
            _file: file,
            size,
            index,
            ctime,
            uuid: header.uuid,
            index_csum: header.index_csum,
        })
    }

    #[inline]
    #[allow(clippy::cast_ptr_alignment)]
    pub fn chunk_end(&self, pos: usize) -> u64 {
        if pos >= self.index.len() {
            panic!("chunk index out of range");
        }
        self.index[pos].end()
    }

    #[inline]
    fn chunk_digest(&self, pos: usize) -> &[u8; 32] {
        if pos >= self.index.len() {
            panic!("chunk index out of range");
        }
        &self.index[pos].digest
    }

    pub fn binary_search(
        &self,
        start_idx: usize,
        start: u64,
        end_idx: usize,
        end: u64,
        offset: u64,
    ) -> Result<usize, Error> {
        if (offset >= end) || (offset < start) {
            bail!("offset out of range");
        }

        if end_idx == start_idx {
            return Ok(start_idx); // found
        }
        let middle_idx = (start_idx + end_idx) / 2;
        let middle_end = self.chunk_end(middle_idx);

        if offset < middle_end {
            self.binary_search(start_idx, start, middle_idx, middle_end, offset)
        } else {
            self.binary_search(middle_idx + 1, middle_end, end_idx, end, offset)
        }
    }
}

impl IndexFile for DynamicIndexReader {
    fn index_count(&self) -> usize {
        self.index.len()
    }

    fn index_digest(&self, pos: usize) -> Option<&[u8; 32]> {
        if pos >= self.index.len() {
            None
        } else {
            Some(unsafe { &*(self.chunk_digest(pos).as_ptr() as *const [u8; 32]) })
        }
    }

    fn index_bytes(&self) -> u64 {
        if self.index.is_empty() {
            0
        } else {
            self.chunk_end(self.index.len() - 1)
        }
    }

    fn compute_csum(&self) -> ([u8; 32], u64) {
        let mut csum = openssl::sha::Sha256::new();
        let mut chunk_end = 0;
        for pos in 0..self.index_count() {
            let info = self.chunk_info(pos).unwrap();
            chunk_end = info.range.end;
            csum.update(&chunk_end.to_le_bytes());
            csum.update(&info.digest);
        }
        let csum = csum.finish();
        (csum, chunk_end)
    }

    fn chunk_info(&self, pos: usize) -> Option<ChunkReadInfo> {
        if pos >= self.index.len() {
            return None;
        }
        let start = if pos == 0 {
            0
        } else {
            self.index[pos - 1].end()
        };

        let end = self.index[pos].end();

        Some(ChunkReadInfo {
            range: start..end,
            digest: self.index[pos].digest,
        })
    }

    fn index_ctime(&self) -> i64 {
        self.ctime
    }

    fn index_size(&self) -> usize {
        self.size
    }

    fn chunk_from_offset(&self, offset: u64) -> Option<(usize, u64)> {
        let end_idx = self.index.len() - 1;
        let end = self.chunk_end(end_idx);
        let found_idx = self.binary_search(0, 0, end_idx, end, offset);
        let found_idx = match found_idx {
            Ok(i) => i,
            Err(_) => return None,
        };

        let found_start = if found_idx == 0 {
            0
        } else {
            self.chunk_end(found_idx - 1)
        };

        Some((found_idx, offset - found_start))
    }
}

/// Create dynamic index files (`.dixd`)
pub struct DynamicIndexWriter {
    store: Arc<ChunkStore>,
    _lock: ProcessLockSharedGuard,
    writer: BufWriter<File>,
    closed: bool,
    filename: PathBuf,
    tmp_filename: PathBuf,
    csum: Option<openssl::sha::Sha256>,
    pub uuid: [u8; 16],
    pub ctime: i64,
}

impl Drop for DynamicIndexWriter {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.tmp_filename); // ignore errors
    }
}

impl DynamicIndexWriter {
    pub fn create(store: Arc<ChunkStore>, path: &Path) -> Result<Self, Error> {
        let shared_lock = store.try_shared_lock()?;

        let full_path = store.relative_path(path);
        let mut tmp_path = full_path.clone();
        tmp_path.set_extension("tmp_didx");

        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .read(true)
            .write(true)
            .open(&tmp_path)?;

        let mut writer = BufWriter::with_capacity(1024 * 1024, file);

        let ctime = proxmox_time::epoch_i64();

        let uuid = Uuid::generate();

        let mut header = DynamicIndexHeader::zeroed();
        header.magic = file_formats::DYNAMIC_SIZED_CHUNK_INDEX_1_0;
        header.ctime = i64::to_le(ctime);
        header.uuid = *uuid.as_bytes();
        // header.index_csum = [0u8; 32];
        writer.write_all(header.as_bytes())?;

        let csum = Some(openssl::sha::Sha256::new());

        Ok(Self {
            store,
            _lock: shared_lock,
            writer,
            closed: false,
            filename: full_path,
            tmp_filename: tmp_path,
            ctime,
            uuid: *uuid.as_bytes(),
            csum,
        })
    }

    // fixme: use add_chunk instead?
    pub fn insert_chunk(&self, chunk: &DataBlob, digest: &[u8; 32]) -> Result<(bool, u64), Error> {
        self.store.insert_chunk(chunk, digest)
    }

    pub fn close(&mut self) -> Result<[u8; 32], Error> {
        if self.closed {
            bail!(
                "cannot close already closed archive index file {:?}",
                self.filename
            );
        }

        self.closed = true;

        self.writer.flush()?;

        let csum_offset = proxmox_lang::offsetof!(DynamicIndexHeader, index_csum);
        self.writer.seek(SeekFrom::Start(csum_offset as u64))?;

        let csum = self.csum.take().unwrap();
        let index_csum = csum.finish();

        self.writer.write_all(&index_csum)?;
        self.writer.flush()?;

        if let Err(err) = std::fs::rename(&self.tmp_filename, &self.filename) {
            bail!("Atomic rename file {:?} failed - {}", self.filename, err);
        }

        Ok(index_csum)
    }

    // fixme: rename to add_digest
    pub fn add_chunk(&mut self, offset: u64, digest: &[u8; 32]) -> Result<(), Error> {
        if self.closed {
            bail!(
                "cannot write to closed dynamic index file {:?}",
                self.filename
            );
        }

        let offset_le: [u8; 8] = offset.to_le().to_ne_bytes();

        if let Some(ref mut csum) = self.csum {
            csum.update(&offset_le);
            csum.update(digest);
        }

        self.writer.write_all(&offset_le)?;
        self.writer.write_all(digest)?;
        Ok(())
    }
}

/// Writer which splits a binary stream into dynamic sized chunks
///
/// And store the resulting chunk list into the index file.
pub struct DynamicChunkWriter {
    index: DynamicIndexWriter,
    closed: bool,
    chunker: Chunker,
    stat: ChunkStat,
    chunk_offset: usize,
    last_chunk: usize,
    chunk_buffer: Vec<u8>,
}

impl DynamicChunkWriter {
    pub fn new(index: DynamicIndexWriter, chunk_size: usize) -> Self {
        Self {
            index,
            closed: false,
            chunker: Chunker::new(chunk_size),
            stat: ChunkStat::new(0),
            chunk_offset: 0,
            last_chunk: 0,
            chunk_buffer: Vec::with_capacity(chunk_size * 4),
        }
    }

    pub fn stat(&self) -> &ChunkStat {
        &self.stat
    }

    pub fn close(&mut self) -> Result<(), Error> {
        if self.closed {
            return Ok(());
        }

        self.closed = true;

        self.write_chunk_buffer()?;

        self.index.close()?;

        self.stat.size = self.chunk_offset as u64;

        // add size of index file
        self.stat.size +=
            (self.stat.chunk_count * 40 + std::mem::size_of::<DynamicIndexHeader>()) as u64;

        Ok(())
    }

    fn write_chunk_buffer(&mut self) -> Result<(), Error> {
        let chunk_size = self.chunk_buffer.len();

        if chunk_size == 0 {
            return Ok(());
        }

        let expected_chunk_size = self.chunk_offset - self.last_chunk;
        if expected_chunk_size != self.chunk_buffer.len() {
            bail!("wrong chunk size {} != {}", expected_chunk_size, chunk_size);
        }

        self.stat.chunk_count += 1;

        self.last_chunk = self.chunk_offset;

        let (chunk, digest) = DataChunkBuilder::new(&self.chunk_buffer)
            .compress(true)
            .build()?;

        match self.index.insert_chunk(&chunk, &digest) {
            Ok((is_duplicate, compressed_size)) => {
                self.stat.compressed_size += compressed_size;
                if is_duplicate {
                    self.stat.duplicate_chunks += 1;
                } else {
                    self.stat.disk_size += compressed_size;
                }

                log::info!(
                    "ADD CHUNK {:016x} {} {}% {} {}",
                    self.chunk_offset,
                    chunk_size,
                    (compressed_size * 100) / (chunk_size as u64),
                    is_duplicate,
                    hex::encode(digest)
                );
                self.index.add_chunk(self.chunk_offset as u64, &digest)?;
                self.chunk_buffer.truncate(0);
                Ok(())
            }
            Err(err) => {
                self.chunk_buffer.truncate(0);
                Err(err)
            }
        }
    }
}

impl Write for DynamicChunkWriter {
    fn write(&mut self, data: &[u8]) -> std::result::Result<usize, std::io::Error> {
        let chunker = &mut self.chunker;

        let pos = chunker.scan(data);

        if pos > 0 {
            self.chunk_buffer.extend_from_slice(&data[0..pos]);
            self.chunk_offset += pos;

            if let Err(err) = self.write_chunk_buffer() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    err.to_string(),
                ));
            }
            Ok(pos)
        } else {
            self.chunk_offset += data.len();
            self.chunk_buffer.extend_from_slice(data);
            Ok(data.len())
        }
    }

    fn flush(&mut self) -> std::result::Result<(), std::io::Error> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            "please use close() instead of flush()",
        ))
    }
}

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
        let cached_chunk = self
            .lru_cache
            .access(
                idx,
                &mut ChunkCacher {
                    store: &mut self.store,
                    index: &self.index,
                },
            )?
            .ok_or_else(|| format_err!("chunk not found by cacher"))?;

        // fixme: avoid copy
        self.read_buffer.clear();
        self.read_buffer.extend_from_slice(&cached_chunk.data);

        self.buffered_chunk_idx = idx;

        self.buffered_chunk_start = cached_chunk.range.start;
        //println!("BUFFER {} {}",  self.buffered_chunk_start, end);
        Ok(())
    }

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
    ) -> MaybeReady<std::io::Result<usize>, ReadAtOperation<'a>> {
        use std::io::Read;
        MaybeReady::Ready(tokio::task::block_in_place(move || {
            let mut reader = self.inner.lock().unwrap();
            reader.seek(SeekFrom::Start(offset))?;
            reader.read(buf)
        }))
    }

    fn poll_complete<'a>(
        self: Pin<&'a Self>,
        _op: ReadAtOperation<'a>,
    ) -> MaybeReady<std::io::Result<usize>, ReadAtOperation<'a>> {
        panic!("LocalDynamicReadAt::start_read_at returned Pending");
    }
}
