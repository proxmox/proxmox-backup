use std::io::Read;

use anyhow::{bail, Error};
use endian_trait::Endian;

use proxmox_io::ReadExt;
use proxmox_uuid::Uuid;

use pbs_datastore::DataBlob;
use pbs_tape::{MediaContentHeader, TapeWrite, PROXMOX_TAPE_BLOCK_SIZE};

use crate::tape::file_formats::{
    ChunkArchiveEntryHeader, ChunkArchiveHeader, PROXMOX_BACKUP_CHUNK_ARCHIVE_ENTRY_MAGIC_1_0,
    PROXMOX_BACKUP_CHUNK_ARCHIVE_MAGIC_1_1,
};

/// Writes chunk archives to tape.
///
/// A chunk archive consists of a `MediaContentHeader` followed by a
/// list of chunks entries. Each chunk entry consists of a
/// `ChunkArchiveEntryHeader` followed by the chunk data (`DataBlob`).
///
/// `| MediaContentHeader | ( ChunkArchiveEntryHeader | DataBlob )* |`
pub struct ChunkArchiveWriter<'a> {
    writer: Option<Box<dyn TapeWrite + 'a>>,
    bytes_written: usize, // does not include bytes from current writer
    close_on_leom: bool,
}

impl<'a> ChunkArchiveWriter<'a> {
    pub const MAGIC: [u8; 8] = PROXMOX_BACKUP_CHUNK_ARCHIVE_MAGIC_1_1;

    /// Creates a new instance
    pub fn new(
        mut writer: Box<dyn TapeWrite + 'a>,
        store: &str,
        close_on_leom: bool,
    ) -> Result<(Self, Uuid), Error> {
        let archive_header = ChunkArchiveHeader {
            store: store.to_string(),
        };
        let header_data = serde_json::to_string_pretty(&archive_header)?
            .as_bytes()
            .to_vec();

        let header = MediaContentHeader::new(Self::MAGIC, header_data.len() as u32);
        writer.write_header(&header, &header_data)?;

        let me = Self {
            writer: Some(writer),
            bytes_written: 0,
            close_on_leom,
        };

        Ok((me, header.uuid.into()))
    }

    /// Returns the number of bytes written so far.
    pub fn bytes_written(&self) -> usize {
        match self.writer {
            Some(ref writer) => writer.bytes_written(),
            None => self.bytes_written, // finalize sets this
        }
    }

    fn write_all(&mut self, data: &[u8]) -> Result<bool, std::io::Error> {
        match self.writer {
            Some(ref mut writer) => writer.write_all(data),
            None => {
                proxmox_lang::io_bail!("detected write after archive finished - internal error")
            }
        }
    }

    /// Write chunk into archive.
    ///
    /// This may return false when `LEOM` is detected (when close_on_leom is set).
    /// In that case the archive only contains parts of the last chunk.
    pub fn try_write_chunk(
        &mut self,
        digest: &[u8; 32],
        blob: &DataBlob,
    ) -> Result<bool, std::io::Error> {
        if self.writer.is_none() {
            return Ok(false);
        }

        let head = ChunkArchiveEntryHeader {
            magic: PROXMOX_BACKUP_CHUNK_ARCHIVE_ENTRY_MAGIC_1_0,
            digest: *digest,
            size: blob.raw_size(),
        };

        let head = head.to_le();
        let data = unsafe {
            std::slice::from_raw_parts(
                &head as *const ChunkArchiveEntryHeader as *const u8,
                std::mem::size_of::<ChunkArchiveEntryHeader>(),
            )
        };

        self.write_all(data)?;

        let mut start = 0;
        let blob_data = blob.raw_data();
        loop {
            if start >= blob_data.len() {
                break;
            }

            let end = start + PROXMOX_TAPE_BLOCK_SIZE;
            let mut chunk_is_complete = false;
            let leom = if end > blob_data.len() {
                chunk_is_complete = true;
                self.write_all(&blob_data[start..])?
            } else {
                self.write_all(&blob_data[start..end])?
            };
            if leom && self.close_on_leom {
                let mut writer = self.writer.take().unwrap();
                writer.finish(false)?;
                self.bytes_written = writer.bytes_written();
                return Ok(chunk_is_complete);
            }
            start = end;
        }

        Ok(true)
    }

    /// This must be called at the end to add padding and `EOF`
    ///
    /// Returns true on `LEOM` or when we hit max archive size
    pub fn finish(&mut self) -> Result<bool, std::io::Error> {
        match self.writer.take() {
            Some(mut writer) => {
                self.bytes_written = writer.bytes_written();
                writer.finish(false)
            }
            None => Ok(true),
        }
    }
}

/// Read chunk archives.
pub struct ChunkArchiveDecoder<R> {
    reader: R,
}

impl<R: Read> ChunkArchiveDecoder<R> {
    /// Creates a new instance
    pub fn new(reader: R) -> Self {
        Self { reader }
    }

    /// Allow access to the underlying reader
    pub fn reader(&self) -> &R {
        &self.reader
    }

    /// Returns the next chunk (if any).
    pub fn next_chunk(&mut self) -> Result<Option<([u8; 32], DataBlob)>, Error> {
        let mut header = ChunkArchiveEntryHeader {
            magic: [0u8; 8],
            digest: [0u8; 32],
            size: 0,
        };
        let data = unsafe {
            std::slice::from_raw_parts_mut(
                (&mut header as *mut ChunkArchiveEntryHeader) as *mut u8,
                std::mem::size_of::<ChunkArchiveEntryHeader>(),
            )
        };

        match self.reader.read_exact_or_eof(data) {
            Ok(true) => {}
            Ok(false) => {
                // last chunk is allowed to be incomplete - simply report EOD
                return Ok(None);
            }
            Err(err) => return Err(err.into()),
        };

        if header.magic != PROXMOX_BACKUP_CHUNK_ARCHIVE_ENTRY_MAGIC_1_0 {
            bail!("wrong magic number");
        }

        let raw_data = match self.reader.read_exact_allocated(header.size as usize) {
            Ok(data) => data,
            Err(err) if err.kind() == std::io::ErrorKind::UnexpectedEof => {
                // last chunk is allowed to be incomplete - simply report EOD
                return Ok(None);
            }
            Err(err) => return Err(err.into()),
        };

        let blob = DataBlob::from_raw(raw_data)?;
        blob.verify_crc()?;

        Ok(Some((header.digest, blob)))
    }
}
