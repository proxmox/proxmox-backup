//! This module implements the proxmox backup chunked data storage
//!
//! A chunk is simply defined as binary blob. We store them inside a
//! `ChunkStore`, addressed by the SHA256 digest of the binary
//! blob. This technology is also known as content-addressable
//! storage.
//!
//! We store larger files by splitting them into chunks. The resulting
//! SHA256 digest list is stored as separate index file. The
//! `DynamicIndex*` format is able to deal with dynamic chunk sizes,
//! whereas the `FixedIndex*` format is an optimization to store a
//! list of equal sized chunks.
//!
//! # ChunkStore Locking
//!
//! We need to be able to restart the proxmox-backup service daemons,
//! so that we can update the software without rebooting the host. But
//! such restarts must not abort running backup jobs, so we need to
//! keep the old service running until those jobs are finished. This
//! implies that we need some kind of locking for the
//! ChunkStore. Please note that it is perfectly valid to have
//! multiple parallel ChunkStore writers, even when they write the
//! same chunk (because the chunk would have the same name and the
//! same data). The only real problem is garbage collection, because
//! we need to avoid deleting chunks which are still referenced.
//!
//! * Read Index Files:
//!
//!   Acquire shared lock for .idx files.
//!
//!
//! * Delete Index Files:
//!
//!   Acquire exclusive lock for .idx files. This makes sure that we do
//!   not delete index files while they are still in use.
//!
//!
//! * Create Index Files:
//!
//!   Acquire shared lock for ChunkStore (process wide).
//!
//!   Note: When creating .idx files, we create temporary (.tmp) file,
//!   then do an atomic rename ...
//!
//!
//! * Garbage Collect:
//!
//!   Acquire exclusive lock for ChunkStore (process wide). If we have
//!   already an shared lock for ChunkStore, try to updraged that
//!   lock.
//!
//!
//! * Server Restart
//!
//!   Try to abort running garbage collection to release exclusive
//!   ChunkStore lock asap. Start new service with existing listening
//!   socket.
//!
//!
//! # Garbage Collection (GC)
//!
//! Deleting backups is as easy as deleting the corresponding .idx
//! files. Unfortunately, this does not free up any storage, because
//! those files just contains references to chunks.
//!
//! To free up some storage, we run a garbage collection process at
//! regular intervals. The collector uses an mark and sweep
//! approach. In the first phase, it scans all .idx files to mark used
//! chunks. The second phase then removes all unmarked chunks from the
//! store.
//!
//! The above locking mechanism makes sure that we are the only
//! process running GC. But we still want to be able to create backups
//! during GC, so there may be multiple backup threads/tasks
//! running. Either started before GC started, or started while GC is
//! running.
//!
//! ## `atime` based GC
//!
//! The idea here is to mark chunks by updating the `atime` (access
//! timestamp) on the chunk file. This is quite simple and does not
//! need additional RAM.
//!
//! One minor problem is that recent Linux versions use the `relatime`
//! mount flag by default for performance reasons (yes, we want
//! that). When enabled, `atime` data is written to the disk only if
//! the file has been modified since the `atime` data was last updated
//! (`mtime`), or if the file was last accessed more than a certain
//! amount of time ago (by default 24h). So we may only delete chunks
//! with `atime` older than 24 hours.
//!
//! Another problem arise from running backups. The mark phase does
//! not find any chunks from those backups, because there is no .idx
//! file for them (created after the backup). Chunks created or
//! touched by those backups may have an `atime` as old as the start
//! time of those backup. Please not that the backup start time may
//! predate the GC start time. Se we may only delete chunk older than
//! the start time of those running backup jobs.
//!
//!
//! ## Store `marks` in RAM using a HASH
//!
//! Not sure if this is better. TODO

#[macro_export]
macro_rules! PROXMOX_BACKUP_PROTOCOL_ID_V1 {
    () =>  { "proxmox-backup-protocol-v1" }
}

// WARNING: PLEASE DO NOT MODIFY THOSE MAGIC VALUES

// openssl::sha::sha256(b"Proxmox Backup uncompressed chunk v1.0")[0..8]
pub static UNCOMPRESSED_CHUNK_MAGIC_1_0: [u8; 8] = [79, 127, 200, 4, 121, 74, 135, 239];

// openssl::sha::sha256(b"Proxmox Backup encrypted chunk v1.0")[0..8]
pub static ENCRYPTED_CHUNK_MAGIC_1_0: [u8; 8] = [8, 54, 114, 153, 70, 156, 26, 151];

// openssl::sha::sha256(b"Proxmox Backup zstd compressed chunk v1.0")[0..8]
pub static COMPRESSED_CHUNK_MAGIC_1_0: [u8; 8] = [191, 237, 46, 195, 108, 17, 228, 235];

// openssl::sha::sha256(b"Proxmox Backup zstd compressed encrypted chunk v1.0")[0..8]
pub static ENCR_COMPR_CHUNK_MAGIC_1_0: [u8; 8] = [9, 40, 53, 200, 37, 150, 90, 196];

// openssl::sha::sha256(b"Proxmox Backup fixed sized chunk index v1.0")[0..8]
pub static FIXED_SIZED_CHUNK_INDEX_1_0: [u8; 8] = [47, 127, 65, 237, 145, 253, 15, 205];

// openssl::sha::sha256(b"Proxmox Backup dynamic sized chunk index v1.0")[0..8]
pub static DYNAMIC_SIZED_CHUNK_INDEX_1_0: [u8; 8] = [28, 145, 78, 165, 25, 186, 179, 205];

mod crypt_config;
pub use crypt_config::*;

mod key_derivation;
pub use key_derivation::*;

mod data_chunk;
pub use data_chunk::*;

mod chunk_stream;
pub use chunk_stream::*;

mod chunk_stat;
pub use chunk_stat::*;

pub use proxmox_protocol::Chunker;

mod chunk_store;
pub use chunk_store::*;

mod index;
pub use index::*;

mod fixed_index;
pub use fixed_index::*;

mod dynamic_index;
pub use dynamic_index::*;

mod backup_info;
pub use backup_info::*;

mod datastore;
pub use datastore::*;
