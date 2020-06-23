use anyhow::{bail, format_err, Error};
use std::convert::TryInto;
use std::io::{Seek, SeekFrom};

use super::chunk_stat::*;
use super::chunk_store::*;
use super::IndexFile;
use crate::tools::{self, epoch_now_u64};

use chrono::{Local, TimeZone};
use std::fs::File;
use std::io::Write;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::read_chunk::*;
use super::ChunkInfo;

use proxmox::tools::io::ReadExt;
use proxmox::tools::Uuid;

/// Header format definition for fixed index files (`.fidx`)
#[repr(C)]
pub struct FixedIndexHeader {
    pub magic: [u8; 8],
    pub uuid: [u8; 16],
    pub ctime: u64,
    /// Sha256 over the index ``SHA256(digest1||digest2||...)``
    pub index_csum: [u8; 32],
    pub size: u64,
    pub chunk_size: u64,
    reserved: [u8; 4016], // overall size is one page (4096 bytes)
}
proxmox::static_assert_size!(FixedIndexHeader, 4096);

// split image into fixed size chunks

pub struct FixedIndexReader {
    _file: File,
    pub chunk_size: usize,
    pub size: u64,
    index_length: usize,
    index: *mut u8,
    pub uuid: [u8; 16],
    pub ctime: u64,
    pub index_csum: [u8; 32],
}

// `index` is mmap()ed which cannot be thread-local so should be sendable
unsafe impl Send for FixedIndexReader {}
unsafe impl Sync for FixedIndexReader {}

impl Drop for FixedIndexReader {
    fn drop(&mut self) {
        if let Err(err) = self.unmap() {
            eprintln!("Unable to unmap file - {}", err);
        }
    }
}

impl FixedIndexReader {
    pub fn open(path: &Path) -> Result<Self, Error> {
        File::open(path)
            .map_err(Error::from)
            .and_then(|file| Self::new(file))
            .map_err(|err| format_err!("Unable to open fixed index {:?} - {}", path, err))
    }

    pub fn new(mut file: std::fs::File) -> Result<Self, Error> {
        if let Err(err) =
            nix::fcntl::flock(file.as_raw_fd(), nix::fcntl::FlockArg::LockSharedNonblock)
        {
            bail!("unable to get shared lock - {}", err);
        }

        file.seek(SeekFrom::Start(0))?;

        let header_size = std::mem::size_of::<FixedIndexHeader>();
        let header: Box<FixedIndexHeader> = unsafe { file.read_host_value_boxed()? };

        if header.magic != super::FIXED_SIZED_CHUNK_INDEX_1_0 {
            bail!("got unknown magic number");
        }

        let size = u64::from_le(header.size);
        let ctime = u64::from_le(header.ctime);
        let chunk_size = u64::from_le(header.chunk_size);

        let index_length = ((size + chunk_size - 1) / chunk_size) as usize;
        let index_size = index_length * 32;

        let rawfd = file.as_raw_fd();

        let stat = match nix::sys::stat::fstat(rawfd) {
            Ok(stat) => stat,
            Err(err) => bail!("fstat failed - {}", err),
        };

        let expected_index_size = (stat.st_size as usize) - header_size;
        if index_size != expected_index_size {
            bail!(
                "got unexpected file size ({} != {})",
                index_size,
                expected_index_size
            );
        }

        let data = unsafe {
            nix::sys::mman::mmap(
                std::ptr::null_mut(),
                index_size,
                nix::sys::mman::ProtFlags::PROT_READ,
                nix::sys::mman::MapFlags::MAP_PRIVATE,
                file.as_raw_fd(),
                header_size as i64,
            )
        }? as *mut u8;

        Ok(Self {
            _file: file,
            chunk_size: chunk_size as usize,
            size,
            index_length,
            index: data,
            ctime,
            uuid: header.uuid,
            index_csum: header.index_csum,
        })
    }

    fn unmap(&mut self) -> Result<(), Error> {
        if self.index == std::ptr::null_mut() {
            return Ok(());
        }

        let index_size = self.index_length * 32;

        if let Err(err) =
            unsafe { nix::sys::mman::munmap(self.index as *mut std::ffi::c_void, index_size) }
        {
            bail!("unmap file failed - {}", err);
        }

        self.index = std::ptr::null_mut();

        Ok(())
    }

    pub fn chunk_info(&self, pos: usize) -> Result<(u64, u64, [u8; 32]), Error> {
        if pos >= self.index_length {
            bail!("chunk index out of range");
        }
        let start = (pos * self.chunk_size) as u64;
        let mut end = start + self.chunk_size as u64;

        if end > self.size {
            end = self.size;
        }

        let mut digest = std::mem::MaybeUninit::<[u8; 32]>::uninit();
        unsafe {
            std::ptr::copy_nonoverlapping(
                self.index.add(pos * 32),
                (*digest.as_mut_ptr()).as_mut_ptr(),
                32,
            );
        }

        Ok((start, end, unsafe { digest.assume_init() }))
    }

    #[inline]
    fn chunk_digest(&self, pos: usize) -> &[u8; 32] {
        if pos >= self.index_length {
            panic!("chunk index out of range");
        }
        let slice = unsafe { std::slice::from_raw_parts(self.index.add(pos * 32), 32) };
        slice.try_into().unwrap()
    }

    #[inline]
    fn chunk_end(&self, pos: usize) -> u64 {
        if pos >= self.index_length {
            panic!("chunk index out of range");
        }

        let end = ((pos + 1) * self.chunk_size) as u64;
        if end > self.size {
            self.size
        } else {
            end
        }
    }

    /// Compute checksum and data size
    pub fn compute_csum(&self) -> ([u8; 32], u64) {
        let mut csum = openssl::sha::Sha256::new();
        let mut chunk_end = 0;
        for pos in 0..self.index_length {
            chunk_end = self.chunk_end(pos);
            let digest = self.chunk_digest(pos);
            csum.update(digest);
        }
        let csum = csum.finish();

        (csum, chunk_end)
    }

    pub fn print_info(&self) {
        println!("Size: {}", self.size);
        println!("ChunkSize: {}", self.chunk_size);
        println!(
            "CTime: {}",
            Local.timestamp(self.ctime as i64, 0).format("%c")
        );
        println!("UUID: {:?}", self.uuid);
    }
}

impl IndexFile for FixedIndexReader {
    fn index_count(&self) -> usize {
        self.index_length
    }

    fn index_digest(&self, pos: usize) -> Option<&[u8; 32]> {
        if pos >= self.index_length {
            None
        } else {
            Some(unsafe { std::mem::transmute(self.index.add(pos * 32)) })
        }
    }

    fn index_bytes(&self) -> u64 {
        self.size
    }
}

pub struct FixedIndexWriter {
    store: Arc<ChunkStore>,
    file: File,
    _lock: tools::ProcessLockSharedGuard,
    filename: PathBuf,
    tmp_filename: PathBuf,
    chunk_size: usize,
    size: usize,
    index_length: usize,
    index: *mut u8,
    pub uuid: [u8; 16],
    pub ctime: u64,
}

// `index` is mmap()ed which cannot be thread-local so should be sendable
unsafe impl Send for FixedIndexWriter {}

impl Drop for FixedIndexWriter {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.tmp_filename); // ignore errors
        if let Err(err) = self.unmap() {
            eprintln!("Unable to unmap file {:?} - {}", self.tmp_filename, err);
        }
    }
}

impl FixedIndexWriter {
    #[allow(clippy::cast_ptr_alignment)]
    pub fn create(
        store: Arc<ChunkStore>,
        path: &Path,
        size: usize,
        chunk_size: usize,
    ) -> Result<Self, Error> {
        let shared_lock = store.try_shared_lock()?;

        let full_path = store.relative_path(path);
        let mut tmp_path = full_path.clone();
        tmp_path.set_extension("tmp_fidx");

        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .read(true)
            .write(true)
            .open(&tmp_path)?;

        let header_size = std::mem::size_of::<FixedIndexHeader>();

        // todo: use static assertion when available in rust
        if header_size != 4096 {
            panic!("got unexpected header size");
        }

        let ctime = epoch_now_u64()?;

        let uuid = Uuid::generate();

        let buffer = vec![0u8; header_size];
        let header = unsafe { &mut *(buffer.as_ptr() as *mut FixedIndexHeader) };

        header.magic = super::FIXED_SIZED_CHUNK_INDEX_1_0;
        header.ctime = u64::to_le(ctime);
        header.size = u64::to_le(size as u64);
        header.chunk_size = u64::to_le(chunk_size as u64);
        header.uuid = *uuid.as_bytes();

        header.index_csum = [0u8; 32];

        file.write_all(&buffer)?;

        let index_length = (size + chunk_size - 1) / chunk_size;
        let index_size = index_length * 32;
        nix::unistd::ftruncate(file.as_raw_fd(), (header_size + index_size) as i64)?;

        let data = unsafe {
            nix::sys::mman::mmap(
                std::ptr::null_mut(),
                index_size,
                nix::sys::mman::ProtFlags::PROT_READ | nix::sys::mman::ProtFlags::PROT_WRITE,
                nix::sys::mman::MapFlags::MAP_SHARED,
                file.as_raw_fd(),
                header_size as i64,
            )
        }? as *mut u8;

        Ok(Self {
            store,
            file,
            _lock: shared_lock,
            filename: full_path,
            tmp_filename: tmp_path,
            chunk_size,
            size,
            index_length,
            index: data,
            ctime,
            uuid: *uuid.as_bytes(),
        })
    }

    pub fn index_length(&self) -> usize {
        self.index_length
    }

    fn unmap(&mut self) -> Result<(), Error> {
        if self.index == std::ptr::null_mut() {
            return Ok(());
        }

        let index_size = self.index_length * 32;

        if let Err(err) =
            unsafe { nix::sys::mman::munmap(self.index as *mut std::ffi::c_void, index_size) }
        {
            bail!("unmap file {:?} failed - {}", self.tmp_filename, err);
        }

        self.index = std::ptr::null_mut();

        Ok(())
    }

    pub fn close(&mut self) -> Result<[u8; 32], Error> {
        if self.index == std::ptr::null_mut() {
            bail!("cannot close already closed index file.");
        }

        let index_size = self.index_length * 32;
        let data = unsafe { std::slice::from_raw_parts(self.index, index_size) };
        let index_csum = openssl::sha::sha256(data);

        self.unmap()?;

        let csum_offset = proxmox::offsetof!(FixedIndexHeader, index_csum);
        self.file.seek(SeekFrom::Start(csum_offset as u64))?;
        self.file.write_all(&index_csum)?;
        self.file.flush()?;

        if let Err(err) = std::fs::rename(&self.tmp_filename, &self.filename) {
            bail!("Atomic rename file {:?} failed - {}", self.filename, err);
        }

        Ok(index_csum)
    }

    pub fn check_chunk_alignment(&self, offset: usize, chunk_len: usize) -> Result<usize, Error> {
        if offset < chunk_len {
            bail!("got chunk with small offset ({} < {}", offset, chunk_len);
        }

        let pos = offset - chunk_len;

        if offset > self.size {
            bail!("chunk data exceeds size ({} >= {})", offset, self.size);
        }

        // last chunk can be smaller
        if ((offset != self.size) && (chunk_len != self.chunk_size))
            || (chunk_len > self.chunk_size)
            || (chunk_len == 0)
        {
            bail!(
                "chunk with unexpected length ({} != {}",
                chunk_len,
                self.chunk_size
            );
        }

        if pos & (self.chunk_size - 1) != 0 {
            bail!("got unaligned chunk (pos = {})", pos);
        }

        Ok(pos / self.chunk_size)
    }

    // Note: We want to add data out of order, so do not assume any order here.
    pub fn add_chunk(&mut self, chunk_info: &ChunkInfo, stat: &mut ChunkStat) -> Result<(), Error> {
        let chunk_len = chunk_info.chunk_len as usize;
        let offset = chunk_info.offset as usize; // end of chunk

        let idx = self.check_chunk_alignment(offset, chunk_len)?;

        let (is_duplicate, compressed_size) = self
            .store
            .insert_chunk(&chunk_info.chunk, &chunk_info.digest)?;

        stat.chunk_count += 1;
        stat.compressed_size += compressed_size;

        let digest = &chunk_info.digest;

        println!(
            "ADD CHUNK {} {} {}% {} {}",
            idx,
            chunk_len,
            (compressed_size * 100) / (chunk_len as u64),
            is_duplicate,
            proxmox::tools::digest_to_hex(digest)
        );

        if is_duplicate {
            stat.duplicate_chunks += 1;
        } else {
            stat.disk_size += compressed_size;
        }

        self.add_digest(idx, digest)
    }

    pub fn add_digest(&mut self, index: usize, digest: &[u8; 32]) -> Result<(), Error> {
        if index >= self.index_length {
            bail!(
                "add digest failed - index out of range ({} >= {})",
                index,
                self.index_length
            );
        }

        if self.index == std::ptr::null_mut() {
            bail!("cannot write to closed index file.");
        }

        let index_pos = index * 32;
        unsafe {
            let dst = self.index.add(index_pos);
            dst.copy_from_nonoverlapping(digest.as_ptr(), 32);
        }

        Ok(())
    }

    pub fn clone_data_from(&mut self, reader: &FixedIndexReader) -> Result<(), Error> {
        if self.index_length != reader.index_count() {
            bail!("clone_data_from failed - index sizes not equal");
        }

        for i in 0..self.index_length {
            self.add_digest(i, reader.index_digest(i).unwrap())?;
        }

        Ok(())
    }
}

pub struct BufferedFixedReader<S> {
    store: S,
    index: FixedIndexReader,
    archive_size: u64,
    read_buffer: Vec<u8>,
    buffered_chunk_idx: usize,
    buffered_chunk_start: u64,
    read_offset: u64,
}

impl<S: ReadChunk> BufferedFixedReader<S> {
    pub fn new(index: FixedIndexReader, store: S) -> Self {
        let archive_size = index.size;
        Self {
            store,
            index,
            archive_size,
            read_buffer: Vec::with_capacity(1024 * 1024),
            buffered_chunk_idx: 0,
            buffered_chunk_start: 0,
            read_offset: 0,
        }
    }

    pub fn archive_size(&self) -> u64 {
        self.archive_size
    }

    fn buffer_chunk(&mut self, idx: usize) -> Result<(), Error> {
        let index = &self.index;
        let (start, end, digest) = index.chunk_info(idx)?;

        // fixme: avoid copy

        let data = self.store.read_chunk(&digest)?;

        if (end - start) != data.len() as u64 {
            bail!(
                "read chunk with wrong size ({} != {}",
                (end - start),
                data.len()
            );
        }

        self.read_buffer.clear();
        self.read_buffer.extend_from_slice(&data);

        self.buffered_chunk_idx = idx;

        self.buffered_chunk_start = start as u64;
        //println!("BUFFER {} {}",  self.buffered_chunk_start, end);
        Ok(())
    }
}

impl<S: ReadChunk> crate::tools::BufferedRead for BufferedFixedReader<S> {
    fn buffered_read(&mut self, offset: u64) -> Result<&[u8], Error> {
        if offset == self.archive_size {
            return Ok(&self.read_buffer[0..0]);
        }

        let buffer_len = self.read_buffer.len();
        let index = &self.index;

        // optimization for sequential read
        if buffer_len > 0
            && ((self.buffered_chunk_idx + 1) < index.index_length)
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
            let idx = (offset / index.chunk_size as u64) as usize;
            self.buffer_chunk(idx)?;
        }

        let buffer_offset = (offset - self.buffered_chunk_start) as usize;
        Ok(&self.read_buffer[buffer_offset..])
    }
}

impl<S: ReadChunk> std::io::Read for BufferedFixedReader<S> {
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

        unsafe {
            std::ptr::copy_nonoverlapping(data.as_ptr(), buf.as_mut_ptr(), n);
        }

        self.read_offset += n as u64;

        Ok(n)
    }
}

impl<S: ReadChunk> Seek for BufferedFixedReader<S> {
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
