use failure::*;

use super::chunk_store::*;

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::os::unix::io::AsRawFd;
use uuid::Uuid;
use chrono::{Local, TimeZone};

#[repr(C)]
pub struct ImageIndexHeader {
    pub magic: [u8; 12],
    pub version: u32,
    pub uuid: [u8; 16],
    pub ctime: u64,
    pub size: u64,
    pub chunk_size: u64,
    reserved: [u8; 4040], // oversall size is one page (4096 bytes)
}

// split image into fixed size chunks

pub struct ImageIndexReader<'a> {
    store: &'a mut ChunkStore,
    filename: PathBuf,
    chunk_size: usize,
    size: usize,
    index: *mut u8,
    uuid: [u8; 16],
    ctime: u64,
}

impl <'a> Drop for ImageIndexReader<'a> {

    fn drop(&mut self) {
        if let Err(err) = self.unmap() {
            eprintln!("Unable to unmap file {:?} - {}", self.filename, err);
        }
    }
}

impl <'a> ImageIndexReader<'a> {

    pub fn open(store: &'a mut ChunkStore, path: &Path) -> Result<Self, Error> {

        let full_path = store.relative_path(path);

        let mut file = std::fs::File::open(&full_path)?;

        let header_size = std::mem::size_of::<ImageIndexHeader>();

        // todo: use static assertion when available in rust
        if header_size != 4096 { panic!("got unexpected header size"); }

        let mut buffer = vec![0u8; header_size];
        file.read_exact(&mut buffer)?;

        let header = unsafe { &mut * (buffer.as_ptr() as *mut ImageIndexHeader) };

        let size = u64::from_be(header.size) as usize;
        let ctime = u64::from_be(header.ctime);
        let chunk_size = u64::from_be(header.chunk_size) as usize;

        let index_size = ((size + chunk_size - 1)/chunk_size)*32;

        // fixme: verify file size ?

        let data = unsafe { nix::sys::mman::mmap(
            std::ptr::null_mut(),
            index_size,
            nix::sys::mman::ProtFlags::PROT_READ,
            nix::sys::mman::MapFlags::MAP_PRIVATE,
            file.as_raw_fd(),
            header_size as i64) }? as *mut u8;

        Ok(Self {
            store,
            filename: full_path,
            chunk_size,
            size,
            index: data,
            ctime,
            uuid: header.uuid,
        })
    }

    fn unmap(&mut self) -> Result<(), Error> {

        if self.index == std::ptr::null_mut() { return Ok(()); }

        let index_size = ((self.size + self.chunk_size - 1)/self.chunk_size)*32;

        if let Err(err) = unsafe { nix::sys::mman::munmap(self.index as *mut std::ffi::c_void, index_size) } {
            bail!("unmap file {:?} failed - {}", self.filename, err);
        }

        self.index = std::ptr::null_mut();

        Ok(())
    }

    pub fn print_info(&self) {
        println!("Filename: {:?}", self.filename);
        println!("Size: {}", self.size);
        println!("ChunkSize: {}", self.chunk_size);
        println!("CTime: {}", Local.timestamp(self.ctime as i64, 0).format("%c"));
        println!("UUID: {:?}", self.uuid);
    }
}

pub struct ImageIndexWriter<'a> {
    store: &'a mut ChunkStore,
    filename: PathBuf,
    tmp_filename: PathBuf,
    chunk_size: usize,
    size: usize,
    index: *mut u8,
    uuid: [u8; 16],
    ctime: u64,
}

impl <'a> Drop for ImageIndexWriter<'a> {

    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.tmp_filename); // ignore errors
        if let Err(err) = self.unmap() {
            eprintln!("Unable to unmap file {:?} - {}", self.tmp_filename, err);
        }
    }
}

impl <'a> ImageIndexWriter<'a> {

    pub fn create(store: &'a mut ChunkStore, path: &Path, size: usize) -> Result<Self, Error> {

        let full_path = store.relative_path(path);
        let mut tmp_path = full_path.clone();
        tmp_path.set_extension("tmp_iidx");

        let mut file = std::fs::OpenOptions::new()
            .create(true).truncate(true)
            .read(true)
            .write(true)
            .open(&tmp_path)?;

        let chunk_size = 64*1024; // fixed size for now??

        let header_size = std::mem::size_of::<ImageIndexHeader>();

        // todo: use static assertion when available in rust
        if header_size != 4096 { panic!("got unexpected header size"); }

        let ctime = std::time::SystemTime::now().duration_since(
            std::time::SystemTime::UNIX_EPOCH)?.as_secs();

        let uuid = Uuid::new_v4();

        let buffer = vec![0u8; header_size];
        let header = unsafe { &mut * (buffer.as_ptr() as *mut ImageIndexHeader) };

        header.magic = *b"PROXMOX-IIDX";
        header.version = u32::to_be(1);
        header.ctime = u64::to_be(ctime);
        header.size = u64::to_be(size as u64);
        header.chunk_size = u64::to_be(chunk_size as u64);
        header.uuid = *uuid.as_bytes();

        file.write_all(&buffer)?;

        let index_size = ((size + chunk_size - 1)/chunk_size)*32;
        nix::unistd::ftruncate(file.as_raw_fd(), (header_size + index_size) as i64)?;

        let data = unsafe { nix::sys::mman::mmap(
            std::ptr::null_mut(),
            index_size,
            nix::sys::mman::ProtFlags::PROT_READ | nix::sys::mman::ProtFlags::PROT_WRITE,
            nix::sys::mman::MapFlags::MAP_SHARED,
            file.as_raw_fd(),
            header_size as i64) }? as *mut u8;


        Ok(Self {
            store,
            filename: full_path,
            tmp_filename: tmp_path,
            chunk_size,
            size,
            index: data,
            ctime,
            uuid: *uuid.as_bytes(),
        })
    }

    fn unmap(&mut self) -> Result<(), Error> {

        if self.index == std::ptr::null_mut() { return Ok(()); }

        let index_size = ((self.size + self.chunk_size - 1)/self.chunk_size)*32;

        if let Err(err) = unsafe { nix::sys::mman::munmap(self.index as *mut std::ffi::c_void, index_size) } {
            bail!("unmap file {:?} failed - {}", self.tmp_filename, err);
        }

        self.index = std::ptr::null_mut();

        Ok(())
    }

    pub fn close(&mut self)  -> Result<(), Error> {

        if self.index == std::ptr::null_mut() { bail!("cannot close already closed index file."); }

        self.unmap()?;

        if let Err(err) = std::fs::rename(&self.tmp_filename, &self.filename) {
            bail!("Atomic rename file {:?} failed - {}", self.filename, err);
        }

        Ok(())
    }

    // Note: We want to add data out of order, so do not assume and order here.
    pub fn add_chunk(&mut self, pos: usize, chunk: &[u8]) -> Result<(), Error> {

        if self.index == std::ptr::null_mut() { bail!("cannot write to closed index file."); }

        let end = pos + chunk.len();

        if end > self.size {
            bail!("write chunk data exceeds size ({} >= {})", end, self.size);
        }

        // last chunk can be smaller
        if ((end != self.size) && (chunk.len() != self.chunk_size)) ||
            (chunk.len() > self.chunk_size) || (chunk.len() == 0) {
                bail!("got chunk with wrong length ({} != {}", chunk.len(), self.chunk_size);
            }

        if pos >= self.size { bail!("add chunk after end ({} >= {})", pos, self.size); }

        if pos & (self.chunk_size-1) != 0 { bail!("add unaligned chunk (pos = {})", pos); }


        let (is_duplicate, digest) = self.store.insert_chunk(chunk)?;

        println!("ADD CHUNK {} {} {} {}", pos, chunk.len(), is_duplicate,  u256_to_hex(&digest));

        let index_pos = (pos/self.chunk_size)*32;
        unsafe {
            let dst = self.index.add(index_pos);
            dst.copy_from_nonoverlapping(digest.as_ptr(), 32);
        }

        Ok(())
    }
}
