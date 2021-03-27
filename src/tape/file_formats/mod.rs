//! File format definitions and implementations for data written to
//! tapes

mod blocked_reader;
pub use blocked_reader::*;

mod blocked_writer;
pub use blocked_writer::*;

mod chunk_archive;
pub use chunk_archive::*;

mod snapshot_archive;
pub use snapshot_archive::*;

mod catalog_archive;
pub use catalog_archive::*;

mod multi_volume_writer;
pub use multi_volume_writer::*;

mod multi_volume_reader;
pub use multi_volume_reader::*;

use std::collections::HashMap;

use anyhow::{bail, Error};
use ::serde::{Deserialize, Serialize};
use endian_trait::Endian;
use bitflags::bitflags;

use proxmox::tools::Uuid;

use crate::backup::Fingerprint;

/// We use 256KB blocksize (always)
pub const PROXMOX_TAPE_BLOCK_SIZE: usize = 256*1024;

// openssl::sha::sha256(b"Proxmox Tape Block Header v1.0")[0..8]
pub const PROXMOX_TAPE_BLOCK_HEADER_MAGIC_1_0: [u8; 8] = [220, 189, 175, 202, 235, 160, 165, 40];

// openssl::sha::sha256(b"Proxmox Backup Content Header v1.0")[0..8];
pub const PROXMOX_BACKUP_CONTENT_HEADER_MAGIC_1_0: [u8; 8] = [99, 238, 20, 159, 205, 242, 155, 12];
// openssl::sha::sha256(b"Proxmox Backup Tape Label v1.0")[0..8];
pub const PROXMOX_BACKUP_MEDIA_LABEL_MAGIC_1_0: [u8; 8] = [42, 5, 191, 60, 176, 48, 170, 57];
// openssl::sha::sha256(b"Proxmox Backup MediaSet Label v1.0")
pub const PROXMOX_BACKUP_MEDIA_SET_LABEL_MAGIC_1_0: [u8; 8] = [8, 96, 99, 249, 47, 151, 83, 216];

// openssl::sha::sha256(b"Proxmox Backup Chunk Archive v1.0")[0..8]
// only used in unreleased version - no longer supported
pub const PROXMOX_BACKUP_CHUNK_ARCHIVE_MAGIC_1_0: [u8; 8] = [62, 173, 167, 95, 49, 76, 6, 110];
// openssl::sha::sha256(b"Proxmox Backup Chunk Archive v1.1")[0..8]
pub const PROXMOX_BACKUP_CHUNK_ARCHIVE_MAGIC_1_1: [u8; 8] = [109, 49, 99, 109, 215, 2, 131, 191];

// openssl::sha::sha256(b"Proxmox Backup Chunk Archive Entry v1.0")[0..8]
pub const PROXMOX_BACKUP_CHUNK_ARCHIVE_ENTRY_MAGIC_1_0: [u8; 8] = [72, 87, 109, 242, 222, 66, 143, 220];

// openssl::sha::sha256(b"Proxmox Backup Snapshot Archive v1.0")[0..8];
// only used in unreleased version - no longer supported
pub const PROXMOX_BACKUP_SNAPSHOT_ARCHIVE_MAGIC_1_0: [u8; 8] = [9, 182, 2, 31, 125, 232, 114, 133];
// openssl::sha::sha256(b"Proxmox Backup Snapshot Archive v1.1")[0..8];
pub const PROXMOX_BACKUP_SNAPSHOT_ARCHIVE_MAGIC_1_1: [u8; 8] = [218, 22, 21, 208, 17, 226, 154, 98];

// openssl::sha::sha256(b"Proxmox Backup Catalog Archive v1.0")[0..8];
pub const PROXMOX_BACKUP_CATALOG_ARCHIVE_MAGIC_1_0: [u8; 8] = [183, 207, 199, 37, 158, 153, 30, 115];

lazy_static::lazy_static!{
    // Map content magic numbers to human readable names.
    static ref PROXMOX_TAPE_CONTENT_NAME: HashMap<&'static [u8;8], &'static str> = {
        let mut map = HashMap::new();
        map.insert(&PROXMOX_BACKUP_MEDIA_LABEL_MAGIC_1_0, "Proxmox Backup Tape Label v1.0");
        map.insert(&PROXMOX_BACKUP_MEDIA_SET_LABEL_MAGIC_1_0, "Proxmox Backup MediaSet Label v1.0");
        map.insert(&PROXMOX_BACKUP_CHUNK_ARCHIVE_MAGIC_1_0, "Proxmox Backup Chunk Archive v1.0");
        map.insert(&PROXMOX_BACKUP_CHUNK_ARCHIVE_MAGIC_1_1, "Proxmox Backup Chunk Archive v1.1");
        map.insert(&PROXMOX_BACKUP_SNAPSHOT_ARCHIVE_MAGIC_1_0, "Proxmox Backup Snapshot Archive v1.0");
        map.insert(&PROXMOX_BACKUP_SNAPSHOT_ARCHIVE_MAGIC_1_1, "Proxmox Backup Snapshot Archive v1.1");
        map.insert(&PROXMOX_BACKUP_CATALOG_ARCHIVE_MAGIC_1_0, "Proxmox Backup Catalog Archive v1.0");
        map
    };
}

/// Map content magic numbers to human readable names.
pub fn proxmox_tape_magic_to_text(magic: &[u8; 8]) -> Option<String> {
    PROXMOX_TAPE_CONTENT_NAME.get(magic).map(|s| String::from(*s))
}

/// Tape Block Header with data payload
///
/// All tape files are written as sequence of blocks.
///
/// Note: this struct is large, never put this on the stack!
/// so we use an unsized type to avoid that.
///
/// Tape data block are always read/written with a fixed size
/// (`PROXMOX_TAPE_BLOCK_SIZE`). But they may contain less data, so the
/// header has an additional size field. For streams of blocks, there
/// is a sequence number (`seq_nr`) which may be use for additional
/// error checking.
#[repr(C,packed)]
pub struct BlockHeader {
    /// fixed value `PROXMOX_TAPE_BLOCK_HEADER_MAGIC_1_0`
    pub magic: [u8; 8],
    pub flags: BlockHeaderFlags,
    /// size as 3 bytes unsigned, little endian
    pub size: [u8; 3],
    /// block sequence number
    pub seq_nr: u32,
    pub payload: [u8],
}

bitflags! {
    /// Header flags (e.g. `END_OF_STREAM` or `INCOMPLETE`)
    pub struct BlockHeaderFlags: u8 {
        /// Marks the last block in a stream.
        const END_OF_STREAM = 0b00000001;
        /// Mark multivolume streams (when set in the last block)
        const INCOMPLETE    = 0b00000010;
    }
}

#[derive(Endian, Copy, Clone, Debug)]
#[repr(C,packed)]
/// Media Content Header
///
/// All tape files start with this header. The header may contain some
/// informational data indicated by `size`.
///
/// `| MediaContentHeader | header data (size) | stream data |`
///
/// Note: The stream data following may be of any size.
pub struct MediaContentHeader {
    /// fixed value `PROXMOX_BACKUP_CONTENT_HEADER_MAGIC_1_0`
    pub magic: [u8; 8],
    /// magic number for the content following
    pub content_magic: [u8; 8],
    /// unique ID to identify this data stream
    pub uuid: [u8; 16],
    /// stream creation time
    pub ctime: i64,
    /// Size of header data
    pub size: u32,
    /// Part number for multipart archives.
    pub part_number: u8,
    /// Reserved for future use
    pub reserved_0: u8,
    /// Reserved for future use
    pub reserved_1: u8,
    /// Reserved for future use
    pub reserved_2: u8,
}

impl MediaContentHeader {

    /// Create a new instance with autogenerated Uuid
    pub fn new(content_magic: [u8; 8], size: u32) -> Self {
        let uuid = *proxmox::tools::uuid::Uuid::generate()
            .into_inner();
        Self {
            magic: PROXMOX_BACKUP_CONTENT_HEADER_MAGIC_1_0,
            content_magic,
            uuid,
            ctime: proxmox::tools::time::epoch_i64(),
            size,
            part_number: 0,
            reserved_0: 0,
            reserved_1: 0,
            reserved_2: 0,
        }
    }

    /// Helper to check magic numbers and size constraints
    pub fn check(&self, content_magic: [u8; 8], min_size: u32, max_size: u32) -> Result<(), Error> {
        if self.magic != PROXMOX_BACKUP_CONTENT_HEADER_MAGIC_1_0 {
            bail!("MediaContentHeader: wrong magic");
        }
        if self.content_magic != content_magic {
            bail!("MediaContentHeader: wrong content magic");
        }
        if self.size < min_size || self.size > max_size {
            bail!("MediaContentHeader: got unexpected size");
        }
        Ok(())
    }

    /// Returns the content Uuid
    pub fn content_uuid(&self) -> Uuid {
        Uuid::from(self.uuid)
    }
}

#[derive(Deserialize, Serialize)]
/// Header for chunk archives
pub struct ChunkArchiveHeader {
    // Datastore name
    pub store: String,
}

#[derive(Endian)]
#[repr(C,packed)]
/// Header for data blobs inside a chunk archive
pub struct ChunkArchiveEntryHeader {
    /// fixed value `PROXMOX_BACKUP_CHUNK_ARCHIVE_ENTRY_MAGIC_1_0`
    pub magic: [u8; 8],
    /// Chunk digest
    pub digest: [u8; 32],
    /// Chunk size
    pub size: u64,
}

#[derive(Deserialize, Serialize)]
/// Header for snapshot archives
pub struct SnapshotArchiveHeader {
    /// Snapshot name
    pub snapshot: String,
    /// Datastore name
    pub store: String,
}

#[derive(Deserialize, Serialize)]
/// Header for Catalog archives
pub struct CatalogArchiveHeader {
    /// The uuid of the media the catalog is for
    pub uuid: Uuid,
    /// The media set uuid the catalog is for
    pub media_set_uuid: Uuid,
    /// Media sequence number
    pub seq_nr: u64,
}

#[derive(Serialize,Deserialize,Clone,Debug)]
/// Media Label
///
/// Media labels are used to uniquely identify a media. They are
/// stored as first file on the tape.
pub struct MediaLabel {
    /// Unique ID
    pub uuid: Uuid,
    /// Media label text (or Barcode)
    pub label_text: String,
    /// Creation time stamp
    pub ctime: i64,
}


#[derive(Serialize,Deserialize,Clone,Debug)]
/// `MediaSet` Label
///
/// Used to uniquely identify a `MediaSet`. They are stored as second
/// file on the tape.
pub struct MediaSetLabel {
    /// The associated `MediaPool`
    pub pool: String,
    /// Uuid. We use the all-zero Uuid to reseve an empty media for a specific pool
    pub uuid: Uuid,
    /// Media sequence number
    pub seq_nr: u64,
    /// Creation time stamp
    pub ctime: i64,
    /// Encryption key finkerprint (if encryped)
    #[serde(skip_serializing_if="Option::is_none")]
    pub encryption_key_fingerprint: Option<Fingerprint>,
}

impl MediaSetLabel {

    pub fn with_data(
        pool: &str,
        uuid: Uuid,
        seq_nr: u64,
        ctime: i64,
        encryption_key_fingerprint: Option<Fingerprint>,
    ) -> Self {
        Self {
            pool: pool.to_string(),
            uuid,
            seq_nr,
            ctime,
            encryption_key_fingerprint,
        }
    }
}

impl BlockHeader {

    pub const SIZE: usize = PROXMOX_TAPE_BLOCK_SIZE;

    /// Allocates a new instance on the heap
    pub fn new() -> Box<Self> {
        use std::alloc::{alloc_zeroed, Layout};

        // align to PAGESIZE, so that we can use it with SG_IO
        let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) } as usize;

        let mut buffer = unsafe {
            let ptr = alloc_zeroed(
                 Layout::from_size_align(Self::SIZE, page_size)
                    .unwrap(),
            );
            Box::from_raw(
                std::slice::from_raw_parts_mut(ptr, Self::SIZE - 16)
                    as *mut [u8] as *mut Self
            )
        };
        buffer.magic = PROXMOX_TAPE_BLOCK_HEADER_MAGIC_1_0;
        buffer
    }

    /// Set the `size` field
    pub fn set_size(&mut self, size: usize) {
        let size = size.to_le_bytes();
        self.size.copy_from_slice(&size[..3]);
    }

    /// Returns the `size` field
    pub fn size(&self) -> usize {
        (self.size[0] as usize) + ((self.size[1] as usize)<<8) + ((self.size[2] as usize)<<16)
    }

    /// Set the `seq_nr` field
    pub fn set_seq_nr(&mut self, seq_nr: u32) {
        self.seq_nr = seq_nr.to_le();
    }

    /// Returns the `seq_nr` field
    pub fn seq_nr(&self) -> u32 {
        u32::from_le(self.seq_nr)
    }
}
