//! File format definitions and implementations for data written to
//! tapes

use std::collections::HashMap;

use endian_trait::Endian;
use serde::{Deserialize, Serialize};

use proxmox_uuid::Uuid;

use pbs_api_types::Fingerprint;

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
pub const PROXMOX_BACKUP_CHUNK_ARCHIVE_ENTRY_MAGIC_1_0: [u8; 8] =
    [72, 87, 109, 242, 222, 66, 143, 220];

// openssl::sha::sha256(b"Proxmox Backup Snapshot Archive v1.0")[0..8];
// only used in unreleased version - no longer supported
pub const PROXMOX_BACKUP_SNAPSHOT_ARCHIVE_MAGIC_1_0: [u8; 8] = [9, 182, 2, 31, 125, 232, 114, 133];
// openssl::sha::sha256(b"Proxmox Backup Snapshot Archive v1.1")[0..8];
pub const PROXMOX_BACKUP_SNAPSHOT_ARCHIVE_MAGIC_1_1: [u8; 8] = [218, 22, 21, 208, 17, 226, 154, 98];
// v1.2 introduced an optional, in-line namespace prefix for the snapshot field
// openssl::sha::sha256(b"Proxmox Backup Snapshot Archive v1.2")[0..8];
pub const PROXMOX_BACKUP_SNAPSHOT_ARCHIVE_MAGIC_1_2: [u8; 8] = [98, 16, 54, 155, 186, 16, 51, 29];

// openssl::sha::sha256(b"Proxmox Backup Catalog Archive v1.0")[0..8];
pub const PROXMOX_BACKUP_CATALOG_ARCHIVE_MAGIC_1_0: [u8; 8] =
    [183, 207, 199, 37, 158, 153, 30, 115];
// v1.1 introduced an optional, in-line namespace prefix for the snapshot field
// openssl::sha::sha256(b"Proxmox Backup Catalog Archive v1.1")[0..8];
pub const PROXMOX_BACKUP_CATALOG_ARCHIVE_MAGIC_1_1: [u8; 8] = [179, 236, 113, 240, 173, 236, 2, 96];

lazy_static::lazy_static! {
    // Map content magic numbers to human readable names.
    static ref PROXMOX_TAPE_CONTENT_NAME: HashMap<&'static [u8;8], &'static str> = {
        let mut map = HashMap::new();
        map.insert(&PROXMOX_BACKUP_MEDIA_LABEL_MAGIC_1_0, "Proxmox Backup Tape Label v1.0");
        map.insert(&PROXMOX_BACKUP_MEDIA_SET_LABEL_MAGIC_1_0, "Proxmox Backup MediaSet Label v1.0");
        map.insert(&PROXMOX_BACKUP_CHUNK_ARCHIVE_MAGIC_1_0, "Proxmox Backup Chunk Archive v1.0");
        map.insert(&PROXMOX_BACKUP_CHUNK_ARCHIVE_MAGIC_1_1, "Proxmox Backup Chunk Archive v1.1");
        map.insert(&PROXMOX_BACKUP_SNAPSHOT_ARCHIVE_MAGIC_1_0, "Proxmox Backup Snapshot Archive v1.0");
        map.insert(&PROXMOX_BACKUP_SNAPSHOT_ARCHIVE_MAGIC_1_1, "Proxmox Backup Snapshot Archive v1.1");
        map.insert(&PROXMOX_BACKUP_SNAPSHOT_ARCHIVE_MAGIC_1_2, "Proxmox Backup Snapshot Archive v1.2");
        map.insert(&PROXMOX_BACKUP_CATALOG_ARCHIVE_MAGIC_1_0, "Proxmox Backup Catalog Archive v1.0");
        map.insert(&PROXMOX_BACKUP_CATALOG_ARCHIVE_MAGIC_1_1, "Proxmox Backup Catalog Archive v1.1");
        map
    };
}

/// Map content magic numbers to human readable names.
pub fn proxmox_tape_magic_to_text(magic: &[u8; 8]) -> Option<String> {
    PROXMOX_TAPE_CONTENT_NAME
        .get(magic)
        .map(|s| String::from(*s))
}

#[derive(Deserialize, Serialize)]
/// Header for chunk archives
pub struct ChunkArchiveHeader {
    // Datastore name
    pub store: String,
}

#[derive(Endian)]
#[repr(C, packed)]
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

#[derive(Serialize, Deserialize, Clone, Debug)]
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
    /// The initial pool the media is reserved for
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pool: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
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
    #[serde(skip_serializing_if = "Option::is_none")]
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

    pub fn new_unassigned(pool: &str, ctime: i64) -> Self {
        Self::with_data(pool, [0u8; 16].into(), 0, ctime, None)
    }

    pub fn unassigned(&self) -> bool {
        self.uuid.as_ref() == [0u8; 16]
    }
}
