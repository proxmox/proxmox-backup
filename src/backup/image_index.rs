use failure::*;

use super::chunk_store::*;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::os::unix::io::AsRawFd;
use uuid::Uuid;

#[repr(C)]
pub struct ImageIndexHeader {
    pub magic: [u8; 12],
    pub version: u32,
    pub uuid: [u8; 16],
    pub ctime: u64,
    pub size: u64,
    reserved: [u8; 4048], // oversall size is one page (4096 bytes)
}

// split image into fixed size chunks

pub struct ImageIndex<'a> {
    store: &'a mut ChunkStore,
    chunk_size: usize,
    size: usize,
    index: *mut u8,
    uuid: [u8; 16],
    ctime: u64,
}

impl <'a> ImageIndex<'a> {

    pub fn create(store: &'a mut ChunkStore, path: &Path, size: usize) -> Result<Self, Error> {

        let full_path = store.relative_path(path);
        println!("FULLPATH: {:?} {}", full_path, size);

        let mut file = std::fs::OpenOptions::new()
            .create(true).truncate(true)
            .read(true)
            .write(true)
            .open(&full_path)?;

        let chunk_size = 64*1024;

        let header_size = std::mem::size_of::<ImageIndexHeader>();

        // todo: use static assertion when available in rust
        if header_size != 4096 { panic!("got unexpected header size"); }

        let ctime = std::time::SystemTime::now().duration_since(
            std::time::SystemTime::UNIX_EPOCH)?.as_secs();

        let uuid = Uuid::new_v4();

        let mut buffer = vec![0u8; header_size];
        let header = unsafe { &mut * (buffer.as_ptr() as *mut ImageIndexHeader) };

        header.magic = *b"PROXMOX-IIDX";
        header.version = u32::to_be(1);
        header.ctime = u64::to_be(ctime);
        header.size = u64::to_be(size as u64);
        header.uuid = *uuid.as_bytes();

        file.write_all(&buffer)?;

        let index_size = ((size + chunk_size - 1)/chunk_size)*32;
        nix::unistd::ftruncate(file.as_raw_fd(), (header_size + index_size) as i64)?;

        println!("SIZES: {} {}", index_size, header_size);

        let data = unsafe { nix::sys::mman::mmap(
            std::ptr::null_mut(),
            index_size,
            nix::sys::mman::ProtFlags::PROT_READ | nix::sys::mman::ProtFlags::PROT_WRITE,
            nix::sys::mman::MapFlags::MAP_SHARED,
            file.as_raw_fd(),
            header_size as i64) }? as *mut u8;


        Ok(Self {
            store,
            chunk_size,
            size,
            index: data,
            ctime,
            uuid: *uuid.as_bytes(),
        })
    }

    // Note: We want to add data out of order, so do not assume and order here.
    pub fn add_chunk(&mut self, pos: usize, chunk: &[u8]) -> Result<(), Error> {

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
