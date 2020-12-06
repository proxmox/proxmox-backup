use anyhow::{bail, Error};
use ::serde::{Deserialize, Serialize};
use endian_trait::Endian;
use bitflags::bitflags;

use proxmox::tools::Uuid;

/// We use 256KB blocksize (always)
pub const PROXMOX_TAPE_BLOCK_SIZE: usize = 256*1024;

// openssl::sha::sha256(b"Proxmox Tape Block Header v1.0")[0..8]
pub const PROXMOX_TAPE_BLOCK_HEADER_MAGIC_1_0: [u8; 8] = [220, 189, 175, 202, 235, 160, 165, 40];

// openssl::sha::sha256(b"Proxmox Backup Content Header v1.0")[0..8];
pub const PROXMOX_BACKUP_CONTENT_HEADER_MAGIC_1_0: [u8; 8] = [99, 238, 20, 159, 205, 242, 155, 12];
// openssl::sha::sha256(b"Proxmox Backup Tape Label v1.0")[0..8];
pub const PROXMOX_BACKUP_DRIVE_LABEL_MAGIC_1_0: [u8; 8] = [42, 5, 191, 60, 176, 48, 170, 57];
// openssl::sha::sha256(b"Proxmox Backup MediaSet Label v1.0")
pub const PROXMOX_BACKUP_MEDIA_SET_LABEL_MAGIC_1_0: [u8; 8] = [8, 96, 99, 249, 47, 151, 83, 216];

// openssl::sha::sha256(b"Proxmox Backup Chunk Archive v1.0")[0..8]
pub const PROXMOX_BACKUP_CHUNK_ARCHIVE_MAGIC_1_0: [u8; 8] = [62, 173, 167, 95, 49, 76, 6, 110];
// openssl::sha::sha256(b"Proxmox Backup Chunk Archive Entry v1.0")[0..8]
pub const PROXMOX_BACKUP_CHUNK_ARCHIVE_ENTRY_MAGIC_1_0: [u8; 8] = [72, 87, 109, 242, 222, 66, 143, 220];

// openssl::sha::sha256(b"Proxmox Backup Snapshot Archive v1.0")[0..8];
pub const PROXMOX_BACKUP_SNAPSHOT_ARCHIVE_MAGIC_1_0: [u8; 8] = [9, 182, 2, 31, 125, 232, 114, 133];

/// Tape Block Header with data payload
///
/// Note: this struct is large, never put this on the stack!
/// so we use an unsized type to avoid that.
///
/// Tape data block are always read/written with a fixed size
/// (PROXMOX_TAPE_BLOCK_SIZE). But they may contain less data, so the
/// header has an additional size field. For streams of blocks, there
/// is a sequence number ('seq_nr') which may be use for additional
/// error checking.
#[repr(C,packed)]
pub struct BlockHeader {
    pub magic: [u8; 8],
    pub flags: BlockHeaderFlags,
    /// size as 3 bytes unsigned, little endian
    pub size: [u8; 3],
    /// block sequence number
    pub seq_nr: u32,
    pub payload: [u8],
}

bitflags! {
    pub struct BlockHeaderFlags: u8 {
        /// Marks the last block in a stream.
        const END_OF_STREAM = 0b00000001;
        /// Mark multivolume streams (when set in the last block)
        const INCOMPLETE    = 0b00000010;
    }
}

#[derive(Endian)]
#[repr(C,packed)]
pub struct ChunkArchiveEntryHeader {
    pub magic: [u8; 8],
    pub digest: [u8; 32],
    pub size: u64,
}

#[derive(Endian, Copy, Clone, Debug)]
#[repr(C,packed)]
pub struct MediaContentHeader {
    pub magic: [u8; 8],
    pub content_magic: [u8; 8],
    pub uuid: [u8; 16],
    pub ctime: i64,
    pub size: u32,
    pub part_number: u8,
    pub reserved_0: u8,
    pub reserved_1: u8,
    pub reserved_2: u8,
}

impl MediaContentHeader {

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

    pub fn content_uuid(&self) -> Uuid {
        Uuid::from(self.uuid)
    }
}

#[derive(Serialize,Deserialize,Clone,Debug)]
pub struct DriveLabel {
    /// Unique ID
    pub uuid: Uuid,
    /// Media Changer ID or Barcode
    pub changer_id: String,
    /// Creation time stamp
    pub ctime: i64,
}


#[derive(Serialize,Deserialize,Clone,Debug)]
pub struct MediaSetLabel {
    pub pool: String,
    /// MediaSet Uuid. We use the all-zero Uuid to reseve an empty media for a specific pool
    pub uuid: Uuid,
    /// MediaSet media sequence number
    pub seq_nr: u64,
    /// Creation time stamp
    pub ctime: i64,
}

impl MediaSetLabel {

    pub fn with_data(pool: &str, uuid: Uuid, seq_nr: u64, ctime: i64) -> Self {
        Self {
            pool: pool.to_string(),
            uuid,
            seq_nr,
            ctime,
        }
    }
}

impl BlockHeader {

    pub const SIZE: usize = PROXMOX_TAPE_BLOCK_SIZE;

    /// Allocates a new instance on the heap
    pub fn new() -> Box<Self> {
        use std::alloc::{alloc_zeroed, Layout};

        let mut buffer = unsafe {
            let ptr = alloc_zeroed(
                Layout::from_size_align(Self::SIZE, std::mem::align_of::<u64>())
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

    pub fn set_size(&mut self, size: usize) {
        let size = size.to_le_bytes();
        self.size.copy_from_slice(&size[..3]);
    }

    pub fn size(&self) -> usize {
        (self.size[0] as usize) + ((self.size[1] as usize)<<8) + ((self.size[2] as usize)<<16)
    }

    pub fn set_seq_nr(&mut self, seq_nr: u32) {
        self.seq_nr = seq_nr.to_le();
    }

    pub fn seq_nr(&self) -> u32 {
        u32::from_le(self.seq_nr)
    }

}
