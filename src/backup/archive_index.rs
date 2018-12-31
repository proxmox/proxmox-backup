use failure::*;

use super::chunk_store::*;
use super::chunker::*;

use std::io::{Read, Write};
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
    reserved: [u8; 4056], // oversall size is one page (4096 bytes)
}

pub struct ArchiveIndexWriter<'a> {
    store: &'a ChunkStore,
    chunker: Chunker,
    file: File,
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

        file.write_all(&buffer)?;

        Ok(Self {
            store,
            chunker: Chunker::new(chunk_size),
            file: file,
            filename: full_path,
            tmp_filename: tmp_path,
            ctime,
            uuid: *uuid.as_bytes(),

            chunk_offset: 0,
            last_chunk: 0,
            chunk_buffer: Vec::with_capacity(chunk_size*4),
        })
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

            let chunk_size = self.chunk_buffer.len();

            let expected_chunk_size = self.chunk_offset - self.last_chunk;
            if expected_chunk_size != self.chunk_buffer.len() {
                panic!("wrong chunk size {} != {}",
                       expected_chunk_size, chunk_size);
            }

            self.last_chunk = self.chunk_offset;

            match self.store.insert_chunk(&self.chunk_buffer) {
                Ok((is_duplicate, digest)) => {
                    println!("ADD CHUNK {} {} {} {}", self.chunk_offset, chunk_size, is_duplicate,  digest_to_hex(&digest));
                    self.chunk_buffer.truncate(0);
                    return Ok(pos);
                }
                Err(err) => {
                    self.chunk_buffer.truncate(0);
                    return Err(Error::new(ErrorKind::Other, err.to_string()));
                }
            }

        } else {
            self.chunk_offset += data.len();
            self.chunk_buffer.extend(data);
            return Ok(data.len());
        }
    }

    fn flush(&mut self) -> std::result::Result<(), std::io::Error> {

        use std::io::{Error, ErrorKind};

        let chunk_size = self.chunk_buffer.len();

        if chunk_size == 0 { return Ok(()); }

        // fixme: finalize index, disable further writes
        match self.store.insert_chunk(&self.chunk_buffer) {
            Ok((is_duplicate, digest)) => {
                println!("ADD LAST CHUNK {} {} {} {}", self.last_chunk, chunk_size, is_duplicate,  digest_to_hex(&digest));
                self.chunk_buffer.truncate(0);
                Ok(())
            }
            Err(err) => {
                self.chunk_buffer.truncate(0);
                Err(Error::new(ErrorKind::Other, err.to_string()))
            }
        }
    }
}
