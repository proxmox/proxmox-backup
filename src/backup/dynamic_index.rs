use std::convert::TryInto;
use std::fs::File;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use failure::*;
use uuid::Uuid;

use proxmox::tools::io::ReadExt;
use proxmox::tools::vec;

use super::Chunker;
use super::IndexFile;
use super::chunk_stat::ChunkStat;
use super::chunk_store::ChunkStore;
use super::read_chunk::ReadChunk;
use super::{DataChunk, DataChunkBuilder};
use crate::tools;

/// Header format definition for dynamic index files (`.dixd`)
#[repr(C)]
pub struct DynamicIndexHeader {
    pub magic: [u8; 8],
    pub uuid: [u8; 16],
    pub ctime: u64,
    /// Sha256 over the index ``SHA256(offset1||digest1||offset2||digest2||...)``
    pub index_csum: [u8; 32],
    reserved: [u8; 4032], // overall size is one page (4096 bytes)
}
proxmox::tools::static_assert_size!(DynamicIndexHeader, 4096);
// TODO: Once non-Copy unions are stabilized, use:
// union DynamicIndexHeader {
//     reserved: [u8; 4096],
//     pub data: DynamicIndexHeaderData,
// }

pub struct DynamicIndexReader {
    _file: File,
    pub size: usize,
    index: *const u8,
    index_entries: usize,
    pub uuid: [u8; 16],
    pub ctime: u64,
    pub index_csum: [u8; 32],
}

// `index` is mmap()ed which cannot be thread-local so should be sendable
// FIXME: Introduce an mmap wrapper type for this?
unsafe impl Send for DynamicIndexReader {}
unsafe impl Sync for DynamicIndexReader {}

impl Drop for DynamicIndexReader {

    fn drop(&mut self) {
        if let Err(err) = self.unmap() {
            eprintln!("Unable to unmap dynamic index - {}", err);
        }
    }
}

impl DynamicIndexReader {

    pub fn open(path: &Path) -> Result<Self, Error> {

        File::open(path)
            .map_err(Error::from)
            .and_then(|file| Self::new(file))
            .map_err(|err| format_err!("Unable to open dynamic index {:?} - {}", path, err))
    }

    pub fn new(mut file: std::fs::File) -> Result<Self, Error> {

        if let Err(err) = nix::fcntl::flock(file.as_raw_fd(), nix::fcntl::FlockArg::LockSharedNonblock) {
            bail!("unable to get shared lock - {}", err);
        }

        file.seek(SeekFrom::Start(0))?;

        let header_size = std::mem::size_of::<DynamicIndexHeader>();

        let buffer = file.read_exact_allocated(header_size)?;

        let header = unsafe { &* (buffer.as_ptr() as *const DynamicIndexHeader) };

        if header.magic != super::DYNAMIC_SIZED_CHUNK_INDEX_1_0 {
            bail!("got unknown magic number");
        }

        let ctime = u64::from_le(header.ctime);

        let rawfd = file.as_raw_fd();

        let stat = nix::sys::stat::fstat(rawfd)?;

        let size = stat.st_size as usize;

        let index_size = size - header_size;
        if (index_size % 40) != 0 {
            bail!("got unexpected file size");
        }

        let data = unsafe { nix::sys::mman::mmap(
            std::ptr::null_mut(),
            index_size,
            nix::sys::mman::ProtFlags::PROT_READ,
            nix::sys::mman::MapFlags::MAP_PRIVATE,
            rawfd,
            header_size as i64) }? as *const u8;

        Ok(Self {
            _file: file,
            size,
            index: data,
            index_entries: index_size/40,
            ctime,
            uuid: header.uuid,
            index_csum: header.index_csum,
        })
    }

    fn unmap(&mut self) -> Result<(), Error> {

        if self.index == std::ptr::null_mut() { return Ok(()); }

        if let Err(err) = unsafe { nix::sys::mman::munmap(self.index as *mut std::ffi::c_void, self.index_entries*40) } {
            bail!("unmap dynamic index failed - {}", err);
        }

        self.index = std::ptr::null_mut();

        Ok(())
    }

    pub fn chunk_info(&self, pos: usize) -> Result<(u64, u64, [u8; 32]), Error> {
        if pos >= self.index_entries {
            bail!("chunk index out of range");
        }
        let start = if pos == 0 {
            0
        } else {
            unsafe { *(self.index.add((pos-1)*40) as *const u64) }
        };

        let end = unsafe { *(self.index.add(pos*40) as *const u64) };

        let mut digest = std::mem::MaybeUninit::<[u8; 32]>::uninit();
        unsafe {
            std::ptr::copy_nonoverlapping(
                self.index.add(pos*40+8),
                (*digest.as_mut_ptr()).as_mut_ptr(),
                32,
            );
        }

        Ok((start, end, unsafe { digest.assume_init() }))
    }

    #[inline]
    fn chunk_end(&self, pos: usize) -> u64 {
        if pos >= self.index_entries {
            panic!("chunk index out of range");
        }
        unsafe { *(self.index.add(pos*40) as *const u64) }
    }

    #[inline]
    fn chunk_digest(&self, pos: usize) -> &[u8; 32] {
        if pos >= self.index_entries {
            panic!("chunk index out of range");
        }
        let slice = unsafe {  std::slice::from_raw_parts(self.index.add(pos*40+8), 32) };
        slice.try_into().unwrap()
    }

    /// Compute checksum and data size
    pub fn compute_csum(&self) -> ([u8; 32], u64) {

        let mut csum = openssl::sha::Sha256::new();
        let mut chunk_end = 0;
        for pos in 0..self.index_entries {
            chunk_end = self.chunk_end(pos);
            let digest = self.chunk_digest(pos);
            csum.update(&chunk_end.to_le_bytes());
            csum.update(digest);
        }
        let csum = csum.finish();

        (csum, chunk_end)
    }

    /*
    pub fn dump_pxar(&self, mut writer: Box<dyn Write>) -> Result<(), Error> {

        for pos in 0..self.index_entries {
            let _end = self.chunk_end(pos);
            let digest = self.chunk_digest(pos);
            //println!("Dump {:08x}", end );
            let chunk = self.store.read_chunk(digest)?;
            // fimxe: handle encrypted chunks
            let data = chunk.decode(None)?;
            writer.write_all(&data)?;
        }

        Ok(())
    }
    */

    fn binary_search(
        &self,
        start_idx: usize,
        start: u64,
        end_idx: usize,
        end: u64,
        offset: u64
    ) -> Result<usize, Error> {

        if (offset >= end) || (offset < start) {
            bail!("offset out of range");
        }

        if end_idx == start_idx {
            return Ok(start_idx); // found
        }
        let middle_idx = (start_idx + end_idx)/2;
        let middle_end = self.chunk_end(middle_idx);

        if offset < middle_end {
            return self.binary_search(start_idx, start, middle_idx, middle_end, offset);
        } else {
            return self.binary_search(middle_idx + 1, middle_end, end_idx, end, offset);
        }
    }
}

impl IndexFile for DynamicIndexReader {
    fn index_count(&self) -> usize {
        self.index_entries
    }

    fn index_digest(&self, pos: usize) -> Option<&[u8; 32]> {
        if pos >= self.index_entries {
            None
        } else {
            Some(unsafe {
                std::mem::transmute(self.chunk_digest(pos).as_ptr())
            })
        }
    }

    fn index_bytes(&self) -> u64 {
        if self.index_entries == 0 {
            0
        } else {
            self.chunk_end((self.index_entries - 1) as usize)
        }
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
}

impl <S: ReadChunk> BufferedDynamicReader<S> {

    pub fn new(index: DynamicIndexReader, store: S) -> Self {

        let archive_size = index.chunk_end(index.index_entries - 1);
        Self {
            store,
            index,
            archive_size,
            read_buffer: Vec::with_capacity(1024*1024),
            buffered_chunk_idx: 0,
            buffered_chunk_start: 0,
            read_offset: 0,
        }
    }

    pub fn archive_size(&self) -> u64 { self.archive_size }

    fn buffer_chunk(&mut self, idx: usize) -> Result<(), Error> {

        let index = &self.index;
        let (start, end, digest) = index.chunk_info(idx)?;

        // fixme: avoid copy

        let data = self.store.read_chunk(&digest)?;

        if (end - start) != data.len() as u64  {
            bail!("read chunk with wrong size ({} != {}", (end - start), data.len());
        }

        self.read_buffer.clear();
        self.read_buffer.extend_from_slice(&data);

        self.buffered_chunk_idx = idx;

        self.buffered_chunk_start = start as u64;
        //println!("BUFFER {} {}",  self.buffered_chunk_start, end);
        Ok(())
    }
}

impl <S: ReadChunk> crate::tools::BufferedRead for BufferedDynamicReader<S> {

    fn buffered_read(&mut self, offset: u64) -> Result<&[u8], Error> {

        if offset == self.archive_size { return Ok(&self.read_buffer[0..0]); }

        let buffer_len = self.read_buffer.len();
        let index = &self.index;

        // optimization for sequential read
        if buffer_len > 0 &&
            ((self.buffered_chunk_idx + 1) < index.index_entries) &&
            (offset >= (self.buffered_chunk_start + (self.read_buffer.len() as u64)))
        {
            let next_idx = self.buffered_chunk_idx + 1;
            let next_end = index.chunk_end(next_idx);
            if offset < next_end {
                self.buffer_chunk(next_idx)?;
                let buffer_offset = (offset - self.buffered_chunk_start) as usize;
                return Ok(&self.read_buffer[buffer_offset..]);
            }
        }

        if (buffer_len == 0) ||
            (offset < self.buffered_chunk_start) ||
            (offset >= (self.buffered_chunk_start + (self.read_buffer.len() as u64)))
        {
            let end_idx = index.index_entries - 1;
            let end = index.chunk_end(end_idx);
            let idx = index.binary_search(0, 0, end_idx, end, offset)?;
            self.buffer_chunk(idx)?;
         }

        let buffer_offset = (offset - self.buffered_chunk_start) as usize;
        Ok(&self.read_buffer[buffer_offset..])
    }
}

impl <S: ReadChunk> std::io::Read for  BufferedDynamicReader<S> {

    fn read(&mut self, buf: &mut [u8]) -> Result<usize, std::io::Error> {

        use std::io::{Error, ErrorKind};
        use crate::tools::BufferedRead;

        let data = match self.buffered_read(self.read_offset) {
            Ok(v) => v,
            Err(err) => return Err(Error::new(ErrorKind::Other, err.to_string())),
        };

        let n = if data.len() > buf.len() { buf.len() } else { data.len() };

        unsafe { std::ptr::copy_nonoverlapping(data.as_ptr(), buf.as_mut_ptr(), n); }

        self.read_offset += n as u64;

        return Ok(n);
    }
}

impl <S: ReadChunk> std::io::Seek for  BufferedDynamicReader<S> {

    fn seek(&mut self, pos: SeekFrom) -> Result<u64, std::io::Error> {

        let new_offset = match pos {
            SeekFrom::Start(start_offset) =>  start_offset as i64,
            SeekFrom::End(end_offset) => (self.archive_size as i64)+ end_offset,
            SeekFrom::Current(offset) => (self.read_offset as i64) + offset,
        };

        use std::io::{Error, ErrorKind};
        if (new_offset < 0) || (new_offset > (self.archive_size as i64)) {
            return Err(Error::new(
                ErrorKind::Other,
                format!("seek is out of range {} ([0..{}])", new_offset, self.archive_size)));
        }
        self.read_offset = new_offset as u64;

        Ok(self.read_offset)
    }
}

/// Create dynamic index files (`.dixd`)
pub struct DynamicIndexWriter {
    store: Arc<ChunkStore>,
    _lock: tools::ProcessLockSharedGuard,
    writer: BufWriter<File>,
    closed: bool,
    filename: PathBuf,
    tmp_filename: PathBuf,
    csum: Option<openssl::sha::Sha256>,
    pub uuid: [u8; 16],
    pub ctime: u64,
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
            .create(true).truncate(true)
            .read(true)
            .write(true)
            .open(&tmp_path)?;

        let mut writer = BufWriter::with_capacity(1024*1024, file);

        let header_size = std::mem::size_of::<DynamicIndexHeader>();

        // todo: use static assertion when available in rust
        if header_size != 4096 { panic!("got unexpected header size"); }

        let ctime = std::time::SystemTime::now().duration_since(
            std::time::SystemTime::UNIX_EPOCH)?.as_secs();

        let uuid = Uuid::new_v4();

        let mut buffer = vec::zeroed(header_size);
        let header = crate::tools::map_struct_mut::<DynamicIndexHeader>(&mut buffer)?;

        header.magic = super::DYNAMIC_SIZED_CHUNK_INDEX_1_0;
        header.ctime = u64::to_le(ctime);
        header.uuid = *uuid.as_bytes();

        header.index_csum = [0u8; 32];

        writer.write_all(&buffer)?;

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
    pub fn insert_chunk(&self, chunk: &DataChunk) -> Result<(bool, u64), Error> {
        self.store.insert_chunk(chunk)
    }

    pub fn close(&mut self)  -> Result<[u8; 32], Error> {

        if self.closed {
            bail!("cannot close already closed archive index file {:?}", self.filename);
        }

        self.closed = true;

        self.writer.flush()?;

        let csum_offset = proxmox::tools::offsetof!(DynamicIndexHeader, index_csum);
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
            bail!("cannot write to closed dynamic index file {:?}", self.filename);
        }

        let offset_le: &[u8; 8] = unsafe { &std::mem::transmute::<u64, [u8;8]>(offset.to_le()) };

        if let Some(ref mut csum) = self.csum {
            csum.update(offset_le);
            csum.update(digest);
        }

        self.writer.write(offset_le)?;
        self.writer.write(digest)?;
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
            chunk_buffer: Vec::with_capacity(chunk_size*4),
        }
    }

    pub fn stat(&self) -> &ChunkStat {
        &self.stat
    }

    pub fn close(&mut self)  -> Result<(), Error> {

        if self.closed {
            return Ok(());
        }

        self.closed = true;

        self.write_chunk_buffer()?;

        self.index.close()?;

        self.stat.size = self.chunk_offset as u64;

        // add size of index file
        self.stat.size += (self.stat.chunk_count*40 + std::mem::size_of::<DynamicIndexHeader>()) as u64;

        Ok(())
    }

    fn write_chunk_buffer(&mut self) -> Result<(), Error> {

        let chunk_size = self.chunk_buffer.len();

        if chunk_size == 0 { return Ok(()); }

        let expected_chunk_size = self.chunk_offset - self.last_chunk;
        if expected_chunk_size != self.chunk_buffer.len() {
            bail!("wrong chunk size {} != {}", expected_chunk_size, chunk_size);
        }

        self.stat.chunk_count += 1;

        self.last_chunk = self.chunk_offset;

        let chunk = DataChunkBuilder::new(&self.chunk_buffer)
            .compress(true)
            .build()?;

        let digest = chunk.digest();

        match self.index.insert_chunk(&chunk) {
            Ok((is_duplicate, compressed_size)) => {

                self.stat.compressed_size += compressed_size;
                if is_duplicate {
                    self.stat.duplicate_chunks += 1;
                } else {
                    self.stat.disk_size += compressed_size;
                }

                println!("ADD CHUNK {:016x} {} {}% {} {}", self.chunk_offset, chunk_size,
                         (compressed_size*100)/(chunk_size as u64), is_duplicate, proxmox::tools::digest_to_hex(digest));
                self.index.add_chunk(self.chunk_offset as u64, &digest)?;
                self.chunk_buffer.truncate(0);
                return Ok(());
            }
            Err(err) => {
                self.chunk_buffer.truncate(0);
                return Err(err);
            }
        }
    }
}

impl Write for DynamicChunkWriter {

    fn write(&mut self, data: &[u8]) -> std::result::Result<usize, std::io::Error> {

        let chunker = &mut self.chunker;

        let pos = chunker.scan(data);

        if pos > 0 {
            self.chunk_buffer.extend(&data[0..pos]);
            self.chunk_offset += pos;

            if let Err(err) = self.write_chunk_buffer() {
                return Err(std::io::Error::new(std::io::ErrorKind::Other, err.to_string()));
            }
            Ok(pos)

        } else {
            self.chunk_offset += data.len();
            self.chunk_buffer.extend(data);
            Ok(data.len())
        }
    }

    fn flush(&mut self) -> std::result::Result<(), std::io::Error> {
        Err(std::io::Error::new(std::io::ErrorKind::Other, "please use close() instead of flush()"))
    }
}
