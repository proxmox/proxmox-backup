use failure::*;

use super::chunk_store::*;
use super::chunker::*;

use std::io::{Read, Write, BufWriter};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::os::unix::io::AsRawFd;
use uuid::Uuid;
use chrono::{Local, TimeZone};

#[repr(C)]
pub struct ArchiveIndexHeader {
    pub magic: [u8; 12],
    pub version: u32,
    pub uuid: [u8; 16],
    pub ctime: u64,
    reserved: [u8; 4056], // overall size is one page (4096 bytes)
}


pub struct ArchiveIndexReader<'a> {
    store: &'a ChunkStore,
    file: File,
    size: usize,
    filename: PathBuf,
    index: *const u8,
    index_entries: usize,
    uuid: [u8; 16],
    ctime: u64,
}

impl <'a> Drop for ArchiveIndexReader<'a> {

    fn drop(&mut self) {
        if let Err(err) = self.unmap() {
            eprintln!("Unable to unmap file {:?} - {}", self.filename, err);
        }
    }
}

impl <'a> ArchiveIndexReader<'a> {

    pub fn open(store: &'a ChunkStore, path: &Path) -> Result<Self, Error> {

        let full_path = store.relative_path(path);

        let mut file = std::fs::File::open(&full_path)?;

        let header_size = std::mem::size_of::<ArchiveIndexHeader>();

        // todo: use static assertion when available in rust
        if header_size != 4096 { bail!("got unexpected header size for {:?}", path); }

        let mut buffer = vec![0u8; header_size];
        file.read_exact(&mut buffer)?;

        let header = unsafe { &mut * (buffer.as_ptr() as *mut ArchiveIndexHeader) };

        if header.magic != *b"PROXMOX-AIDX" {
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

        let index_size = (size - header_size);
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
            file,
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

    pub fn mark_used_chunks(&self, status: &mut GarbageCollectionStatus) -> Result<(), Error> {

        for pos in 0..self.index_entries {
            let offset = unsafe { *(self.index.add(pos*40) as *const u64) };
            let digest = unsafe { std::slice::from_raw_parts(self.index.add(pos*40+8), 32) };

            if let Err(err) = self.store.touch_chunk(digest) {
                bail!("unable to access chunk {}, required by {:?} - {}",
                      digest_to_hex(digest), self.filename, err);
            }
        }
        Ok(())
    }

    pub fn dump_catar(&self, mut writer: Box<Write>) -> Result<(), Error> {

        let mut buffer = Vec::with_capacity(1024*1024);

        for pos in 0..self.index_entries {
            let offset = unsafe { *(self.index.add(pos*40) as *const u64) };
            let digest = unsafe { std::slice::from_raw_parts(self.index.add(pos*40+8), 32) };

            self.store.read_chunk(digest, &mut buffer)?;
            println!("Dump {:08x} {}", offset, buffer.len(), );
            writer.write_all(&buffer)?;

        }

        Ok(())
    }
}


pub struct ArchiveIndexWriter<'a> {
    store: &'a ChunkStore,
    chunker: Chunker,
    writer: BufWriter<File>,
    closed: bool,
    filename: PathBuf,
    tmp_filename: PathBuf,
    uuid: [u8; 16],
    ctime: u64,

    chunk_offset: usize,
    last_chunk: usize,
    chunk_buffer: Vec<u8>,
}

impl <'a> ArchiveIndexWriter<'a> {

    pub fn create(store: &'a ChunkStore, path: &Path, chunk_size: usize) -> Result<Self, Error> {

        let full_path = store.relative_path(path);
        let mut tmp_path = full_path.clone();
        tmp_path.set_extension("tmp_aidx");

        let mut file = std::fs::OpenOptions::new()
            .create(true).truncate(true)
            .read(true)
            .write(true)
            .open(&tmp_path)?;

        let mut writer = BufWriter::with_capacity(1024*1024, file);

        let header_size = std::mem::size_of::<ArchiveIndexHeader>();

        // todo: use static assertion when available in rust
        if header_size != 4096 { panic!("got unexpected header size"); }

        let ctime = std::time::SystemTime::now().duration_since(
            std::time::SystemTime::UNIX_EPOCH)?.as_secs();

        let uuid = Uuid::new_v4();

        let mut buffer = vec![0u8; header_size];
        let header = crate::tools::map_struct_mut::<ArchiveIndexHeader>(&mut buffer)?;

        header.magic = *b"PROXMOX-AIDX";
        header.version = u32::to_le(1);
        header.ctime = u64::to_le(ctime);
        header.uuid = *uuid.as_bytes();

        writer.write_all(&buffer)?;

        Ok(Self {
            store,
            chunker: Chunker::new(chunk_size),
            writer: writer,
            closed: false,
            filename: full_path,
            tmp_filename: tmp_path,
            ctime,
            uuid: *uuid.as_bytes(),

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

        // fixme:

        if let Err(err) = std::fs::rename(&self.tmp_filename, &self.filename) {
            bail!("Atomic rename file {:?} failed - {}", self.filename, err);
        }

        Ok(())
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

        self.last_chunk = self.chunk_offset;

        match self.store.insert_chunk(&self.chunk_buffer) {
            Ok((is_duplicate, digest)) => {
                println!("ADD CHUNK {:016x} {} {} {}", self.chunk_offset, chunk_size, is_duplicate,  digest_to_hex(&digest));
                let chunk_end =
                self.writer.write(unsafe { &std::mem::transmute::<u64, [u8;8]>(self.chunk_offset as u64) })?;
                self.writer.write(&digest)?;
                self.chunk_buffer.truncate(0);
                return Ok(());
            }
            Err(err) => {
                self.chunk_buffer.truncate(0);
                return Err(Error::new(ErrorKind::Other, err.to_string()));
            }
        }

        Ok(())
    }
}

impl <'a> Write for ArchiveIndexWriter<'a> {

    fn write(&mut self, data: &[u8]) -> std::result::Result<usize, std::io::Error> {

        use std::io::{Error, ErrorKind};

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
