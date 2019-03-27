use failure::*;

use crate::tools;
use super::IndexFile;
use super::chunk_stat::*;
use super::chunk_store::*;
use proxmox_protocol::Chunker;

use std::sync::Arc;
use std::io::{Read, Write, BufWriter};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::os::unix::io::AsRawFd;
use uuid::Uuid;
//use chrono::{Local, TimeZone};

/// Header format definition for dynamic index files (`.dixd`)
#[repr(C)]
pub struct DynamicIndexHeader {
    /// The string `PROXMOX-DIDX`
    pub magic: [u8; 12],
    pub version: u32,
    pub uuid: [u8; 16],
    pub ctime: u64,
    reserved: [u8; 4056], // overall size is one page (4096 bytes)
}


pub struct DynamicIndexReader {
    store: Arc<ChunkStore>,
    _file: File,
    pub size: usize,
    filename: PathBuf,
    index: *const u8,
    index_entries: usize,
    pub uuid: [u8; 16],
    pub ctime: u64,
}

// `index` is mmap()ed which cannot be thread-local so should be sendable
// FIXME: Introduce an mmap wrapper type for this?
unsafe impl Send for DynamicIndexReader {}

impl Drop for DynamicIndexReader {

    fn drop(&mut self) {
        if let Err(err) = self.unmap() {
            eprintln!("Unable to unmap file {:?} - {}", self.filename, err);
        }
    }
}

impl DynamicIndexReader {

    pub fn open(store: Arc<ChunkStore>, path: &Path) -> Result<Self, Error> {

        let full_path = store.relative_path(path);

        let mut file = std::fs::File::open(&full_path)?;

        if let Err(err) = nix::fcntl::flock(file.as_raw_fd(), nix::fcntl::FlockArg::LockSharedNonblock) {
            bail!("unable to get shared lock on {:?} - {}", full_path, err);
        }

        let header_size = std::mem::size_of::<DynamicIndexHeader>();

        // todo: use static assertion when available in rust
        if header_size != 4096 { bail!("got unexpected header size for {:?}", path); }

        let mut buffer = vec![0u8; header_size];
        file.read_exact(&mut buffer)?;

        let header = unsafe { &mut * (buffer.as_ptr() as *mut DynamicIndexHeader) };

        if header.magic != *b"PROXMOX-DIDX" {
            bail!("got unknown magic number for {:?}", path);
        }

        let version = u32::from_le(header.version);
        if  version != 1 {
            bail!("got unsupported version number ({}) for {:?}", version, path);
        }

        let ctime = u64::from_le(header.ctime);

        let rawfd = file.as_raw_fd();

        let stat = match nix::sys::stat::fstat(rawfd) {
            Ok(stat) => stat,
            Err(err) => bail!("fstat {:?} failed - {}", path, err),
        };

        let size = stat.st_size as usize;

        let index_size = size - header_size;
        if (index_size % 40) != 0 {
            bail!("got unexpected file size for {:?}", path);
        }

        let data = unsafe { nix::sys::mman::mmap(
            std::ptr::null_mut(),
            index_size,
            nix::sys::mman::ProtFlags::PROT_READ,
            nix::sys::mman::MapFlags::MAP_PRIVATE,
            rawfd,
            header_size as i64) }? as *const u8;

        Ok(Self {
            store,
            filename: full_path,
            _file: file,
            size,
            index: data,
            index_entries: index_size/40,
            ctime,
            uuid: header.uuid,
        })
    }

    fn unmap(&mut self) -> Result<(), Error> {

        if self.index == std::ptr::null_mut() { return Ok(()); }

        if let Err(err) = unsafe { nix::sys::mman::munmap(self.index as *mut std::ffi::c_void, self.index_entries*40) } {
            bail!("unmap file {:?} failed - {}", self.filename, err);
        }

        self.index = std::ptr::null_mut();

        Ok(())
    }

    #[inline]
    fn chunk_end(&self, pos: usize) -> u64 {
        if pos >= self.index_entries {
            panic!("chunk index out of range");
        }
        unsafe { *(self.index.add(pos*40) as *const u64) }
    }

    #[inline]
    fn chunk_digest(&self, pos: usize) -> &[u8] {
        if pos >= self.index_entries {
            panic!("chunk index out of range");
        }
        unsafe {  std::slice::from_raw_parts(self.index.add(pos*40+8), 32) }
    }

    pub fn mark_used_chunks(&self, _status: &mut GarbageCollectionStatus) -> Result<(), Error> {

        for pos in 0..self.index_entries {
            let digest = self.chunk_digest(pos);
            if let Err(err) = self.store.touch_chunk(digest) {
                bail!("unable to access chunk {}, required by {:?} - {}",
                      tools::digest_to_hex(digest), self.filename, err);
            }
        }
        Ok(())
    }

    pub fn dump_pxar(&self, mut writer: Box<Write>) -> Result<(), Error> {

        let mut buffer = Vec::with_capacity(1024*1024);

        for pos in 0..self.index_entries {
            let _end = self.chunk_end(pos);
            let digest = self.chunk_digest(pos);
            //println!("Dump {:08x}", end );
            self.store.read_chunk(digest, &mut buffer)?;
            writer.write_all(&buffer)?;

        }

        Ok(())
    }

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
}

pub struct BufferedDynamicReader {
    index: DynamicIndexReader,
    archive_size: u64,
    read_buffer: Vec<u8>,
    buffered_chunk_idx: usize,
    buffered_chunk_start: u64,
    read_offset: u64,
}

impl BufferedDynamicReader {

    pub fn new(index: DynamicIndexReader) -> Self {

        let archive_size = index.chunk_end(index.index_entries - 1);
        Self {
            index: index,
            archive_size: archive_size,
            read_buffer: Vec::with_capacity(1024*1024),
            buffered_chunk_idx: 0,
            buffered_chunk_start: 0,
            read_offset: 0,
        }
    }

    pub fn archive_size(&self) -> u64 { self.archive_size }

    fn buffer_chunk(&mut self, idx: usize) -> Result<(), Error> {

        let index = &self.index;
        let end = index.chunk_end(idx);
        let digest = index.chunk_digest(idx);
        index.store.read_chunk(digest, &mut self.read_buffer)?;

        self.buffered_chunk_idx = idx;
        self.buffered_chunk_start = end - (self.read_buffer.len() as u64);
        //println!("BUFFER {} {}",  self.buffered_chunk_start, end);
        Ok(())
    }
}

impl crate::tools::BufferedRead for BufferedDynamicReader {

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

impl std::io::Read for  BufferedDynamicReader {

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

impl std::io::Seek for  BufferedDynamicReader {

    fn seek(&mut self, pos: std::io::SeekFrom) -> Result<u64, std::io::Error> {

        use std::io::{SeekFrom};

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

pub struct DynamicIndexWriter {
    store: Arc<ChunkStore>,
    _lock: tools::ProcessLockSharedGuard,

    chunker: Chunker,
    writer: BufWriter<File>,
    closed: bool,
    filename: PathBuf,
    tmp_filename: PathBuf,
    pub uuid: [u8; 16],
    pub ctime: u64,

    stat: ChunkStat,

    chunk_offset: usize,
    last_chunk: usize,
    chunk_buffer: Vec<u8>,
}

impl Drop for DynamicIndexWriter {

    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.tmp_filename); // ignore errors
    }
}

impl DynamicIndexWriter {

    pub fn create(store: Arc<ChunkStore>, path: &Path, chunk_size: usize) -> Result<Self, Error> {

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

        let mut buffer = vec![0u8; header_size];
        let header = crate::tools::map_struct_mut::<DynamicIndexHeader>(&mut buffer)?;

        header.magic = *b"PROXMOX-DIDX";
        header.version = u32::to_le(1);
        header.ctime = u64::to_le(ctime);
        header.uuid = *uuid.as_bytes();

        writer.write_all(&buffer)?;

        Ok(Self {
            store,
            _lock: shared_lock,
            chunker: Chunker::new(chunk_size),
            writer: writer,
            closed: false,
            filename: full_path,
            tmp_filename: tmp_path,
            ctime,
            uuid: *uuid.as_bytes(),

            stat: ChunkStat::new(0),

            chunk_offset: 0,
            last_chunk: 0,
            chunk_buffer: Vec::with_capacity(chunk_size*4),
        })
    }

    pub fn close(&mut self)  -> Result<(), Error> {

        if self.closed {
            bail!("cannot close already closed archive index file {:?}", self.filename);
        }

        self.closed = true;

        self.write_chunk_buffer()?;

        self.writer.flush()?;

        self.stat.size = self.chunk_offset as u64;

        // add size of index file
        self.stat.size += (self.stat.chunk_count*40 + std::mem::size_of::<DynamicIndexHeader>()) as u64;

        println!("STAT: {:?}", self.stat);

        // fixme:

        if let Err(err) = std::fs::rename(&self.tmp_filename, &self.filename) {
            bail!("Atomic rename file {:?} failed - {}", self.filename, err);
        }

        Ok(())
    }

    pub fn stat(&self) -> &ChunkStat {
        &self.stat
    }

    fn write_chunk_buffer(&mut self) -> Result<(), std::io::Error> {

        use std::io::{Error, ErrorKind};

        let chunk_size = self.chunk_buffer.len();

        if chunk_size == 0 { return Ok(()); }

        let expected_chunk_size = self.chunk_offset - self.last_chunk;
        if expected_chunk_size != self.chunk_buffer.len() {
            return Err(Error::new(
                ErrorKind::Other,
                format!("wrong chunk size {} != {}", expected_chunk_size, chunk_size)));
        }

        self.stat.chunk_count += 1;

        self.last_chunk = self.chunk_offset;

        match self.store.insert_chunk(&self.chunk_buffer) {
            Ok((is_duplicate, digest, compressed_size)) => {

                self.stat.compressed_size += compressed_size;
                if is_duplicate {
                    self.stat.duplicate_chunks += 1;
                } else {
                    self.stat.disk_size += compressed_size;
                }

                println!("ADD CHUNK {:016x} {} {}% {} {}", self.chunk_offset, chunk_size,
                         (compressed_size*100)/(chunk_size as u64), is_duplicate,  tools::digest_to_hex(&digest));
                self.add_chunk(self.chunk_offset as u64, &digest)?;
                self.chunk_buffer.truncate(0);
                return Ok(());
            }
            Err(err) => {
                self.chunk_buffer.truncate(0);
                return Err(Error::new(ErrorKind::Other, err.to_string()));
            }
        }
    }

    pub fn add_chunk(&mut self, offset: u64, digest: &[u8; 32]) -> Result<(), std::io::Error> {
        self.writer.write(unsafe { &std::mem::transmute::<u64, [u8;8]>(offset.to_le()) })?;
        self.writer.write(digest)?;
        Ok(())
    }
}

impl Write for DynamicIndexWriter {

    fn write(&mut self, data: &[u8]) -> std::result::Result<usize, std::io::Error> {

        let chunker = &mut self.chunker;

        let pos = chunker.scan(data);

        if pos > 0 {
            self.chunk_buffer.extend(&data[0..pos]);
            self.chunk_offset += pos;

            self.write_chunk_buffer()?;
            Ok(pos)

        } else {
            self.chunk_offset += data.len();
            self.chunk_buffer.extend(data);
            Ok(data.len())
        }
    }

    fn flush(&mut self) -> std::result::Result<(), std::io::Error> {

        use std::io::{Error, ErrorKind};

        Err(Error::new(ErrorKind::Other, "please use close() instead of flush()"))
    }
}
