//! This module implements the proxmox backup data storage
//!
//! Proxmox backup splits large files into chunks, and stores them
//! deduplicated using a content addressable storage format.
//!
//! A chunk is simply defined as binary blob, which is stored inside a
//! `ChunkStore`, addressed by the SHA256 digest of the binary blob.
//!
//! Index files are used to reconstruct the original file. They
//! basically contain a list of SHA256 checksums. The `DynamicIndex*`
//! format is able to deal with dynamic chunk sizes, whereas the
//! `FixedIndex*` format is an optimization to store a list of equal
//! sized chunks.
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
//!   Note: When creating .idx files, we create temporary a (.tmp) file,
//!   then do an atomic rename ...
//!
//!
//! * Garbage Collect:
//!
//!   Acquire exclusive lock for ChunkStore (process wide). If we have
//!   already a shared lock for the ChunkStore, try to upgrade that
//!   lock.
//!
//!
//! * Server Restart
//!
//!   Try to abort the running garbage collection to release exclusive
//!   ChunkStore locks ASAP. Start the new service with the existing listening
//!   socket.
//!
//!
//! # Garbage Collection (GC)
//!
//! Deleting backups is as easy as deleting the corresponding .idx
//! files. Unfortunately, this does not free up any storage, because
//! those files just contain references to chunks.
//!
//! To free up some storage, we run a garbage collection process at
//! regular intervals. The collector uses a mark and sweep
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
//! Another problem arises from running backups. The mark phase does
//! not find any chunks from those backups, because there is no .idx
//! file for them (created after the backup). Chunks created or
//! touched by those backups may have an `atime` as old as the start
//! time of those backups. Please note that the backup start time may
//! predate the GC start time. So we may only delete chunks older than
//! the start time of those running backup jobs.
//!
//!
//! ## Store `marks` in RAM using a HASH
//!
//! Not sure if this is better. TODO

use anyhow::{bail, Error};

// Note: .pcat1 => Proxmox Catalog Format version 1
pub const CATALOG_NAME: &str = "catalog.pcat1.didx";

#[macro_export]
macro_rules! PROXMOX_BACKUP_PROTOCOL_ID_V1 {
    () =>  { "proxmox-backup-protocol-v1" }
}

#[macro_export]
macro_rules! PROXMOX_BACKUP_READER_PROTOCOL_ID_V1 {
    () =>  { "proxmox-backup-reader-protocol-v1" }
}

/// Unix system user used by proxmox-backup-proxy
pub const BACKUP_USER_NAME: &str = "backup";
/// Unix system group used by proxmox-backup-proxy
pub const BACKUP_GROUP_NAME: &str = "backup";

/// Return User info for the 'backup' user (``getpwnam_r(3)``)
pub fn backup_user() -> Result<nix::unistd::User, Error> {
    match nix::unistd::User::from_name(BACKUP_USER_NAME)? {
        Some(user) => Ok(user),
        None => bail!("Unable to lookup backup user."),
    }
}

/// Return Group info for the 'backup' group (``getgrnam(3)``)
pub fn backup_group() -> Result<nix::unistd::Group, Error> {
    match nix::unistd::Group::from_name(BACKUP_GROUP_NAME)? {
        Some(group) => Ok(group),
        None => bail!("Unable to lookup backup user."),
    }
}

mod file_formats;
pub use file_formats::*;

mod manifest;
pub use manifest::*;

mod crypt_config;
pub use crypt_config::*;

mod key_derivation;
pub use key_derivation::*;

mod crypt_reader;
pub use crypt_reader::*;

mod crypt_writer;
pub use crypt_writer::*;

mod checksum_reader;
pub use checksum_reader::*;

mod checksum_writer;
pub use checksum_writer::*;

mod chunker;
pub use chunker::*;

mod data_blob;
pub use data_blob::*;

mod data_blob_reader;
pub use data_blob_reader::*;

mod data_blob_writer;
pub use data_blob_writer::*;

mod catalog;
pub use catalog::*;

mod chunk_stream;
pub use chunk_stream::*;

mod chunk_stat;
pub use chunk_stat::*;

mod read_chunk;
pub use read_chunk::*;

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

mod prune;
pub use prune::*;

mod datastore;
pub use datastore::*;

mod verify;
pub use verify::*;

mod catalog_shell;
pub use catalog_shell::*;

mod async_index_reader;
pub use async_index_reader::*;
