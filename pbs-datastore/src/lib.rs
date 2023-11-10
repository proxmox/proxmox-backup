//! This module implements the data storage and access layer.
//!
//! # Data formats
//!
//! PBS splits large files into chunks, and stores them deduplicated using
//! a content addressable storage format.
//!
//! Backup snapshots are stored as folders containing a manifest file and
//! potentially one or more index or blob files.
//!
//! The manifest contains hashes of all other files and can be signed by
//! the client.
//!
//! Blob files contain data directly. They are used for config files and
//! the like.
//!
//! Index files are used to reconstruct an original file. They contain a
//! list of SHA256 checksums. The `DynamicIndex*` format is able to deal
//! with dynamic chunk sizes (CT and host backups), whereas the
//! `FixedIndex*` format is an optimization to store a list of equal sized
//! chunks (VMs, whole block devices).
//!
//! A chunk is defined as a binary blob, which is stored inside a
//! [ChunkStore] instead of the backup directory
//! directly, and can be addressed by its SHA256 digest.
//!
//!
//! # Garbage Collection (GC)
//!
//! Deleting backups is as easy as deleting the corresponding .idx files.
//! However, this does not free up any storage, because those files just
//! contain references to chunks.
//!
//! To free up some storage, we run a garbage collection process at
//! regular intervals. The collector uses a mark and sweep approach. In
//! the first phase, it scans all .idx files to mark used chunks. The
//! second phase then removes all unmarked chunks from the store.
//!
//! The locking mechanisms mentioned below make sure that we are the only
//! process running GC. We still want to be able to create backups during
//! GC, so there may be multiple backup threads/tasks running, either
//! started before GC, or while GC is running.
//!
//! ## `atime` based GC
//!
//! The idea here is to mark chunks by updating the `atime` (access
//! timestamp) on the chunk file. This is quite simple and does not need
//! additional RAM.
//!
//! One minor problem is that recent Linux versions use the `relatime`
//! mount flag by default for performance reasons (and we want that). When
//! enabled, `atime` data is written to the disk only if the file has been
//! modified since the `atime` data was last updated (`mtime`), or if the
//! file was last accessed more than a certain amount of time ago (by
//! default 24h). So we may only delete chunks with `atime` older than 24
//! hours.
//!
//! Another problem arises from running backups. The mark phase does not
//! find any chunks from those backups, because there is no .idx file for
//! them (created after the backup). Chunks created or touched by those
//! backups may have an `atime` as old as the start time of those backups.
//! Please note that the backup start time may predate the GC start time.
//! So we may only delete chunks older than the start time of those
//! running backup jobs, which might be more than 24h back (this is the
//! reason why ProcessLocker exclusive locks only have to be exclusive
//! between processes, since within one we can determine the age of the
//! oldest shared lock).
//!
//! ## Store `marks` in RAM using a HASH
//!
//! Might be better. Under investigation.
//!
//!
//! # Locking
//!
//! Since PBS allows multiple potentially interfering operations at the
//! same time (e.g. garbage collect, prune, multiple backup creations
//! (only in separate groups), forget, ...), these need to lock against
//! each other in certain scenarios. There is no overarching global lock
//! though, instead always the finest grained lock possible is used,
//! because running these operations concurrently is treated as a feature
//! on its own.
//!
//! ## Inter-process Locking
//!
//! We need to be able to restart the proxmox-backup service daemons, so
//! that we can update the software without rebooting the host. But such
//! restarts must not abort running backup jobs, so we need to keep the
//! old service running until those jobs are finished. This implies that
//! we need some kind of locking for modifying chunks and indices in the
//! ChunkStore.
//!
//! Please note that it is perfectly valid to have multiple
//! parallel ChunkStore writers, even when they write the same chunk
//! (because the chunk would have the same name and the same data, and
//! writes are completed atomically via a rename). The only problem is
//! garbage collection, because we need to avoid deleting chunks which are
//! still referenced.
//!
//! To do this we use the
//! [ProcessLocker](proxmox_sys::process_locker::ProcessLocker).
//!
//! ### ChunkStore-wide
//!
//! * Create Index Files:
//!
//!   Acquire shared lock for ChunkStore.
//!
//!   Note: When creating .idx files, we create a temporary .tmp file,
//!   then do an atomic rename.
//!
//! * Garbage Collect:
//!
//!   Acquire exclusive lock for ChunkStore. If we have
//!   already a shared lock for the ChunkStore, try to upgrade that
//!   lock.
//!
//! Exclusive locks only work _between processes_. It is valid to have an
//! exclusive and one or more shared locks held within one process. Writing
//! chunks within one process is synchronized using the gc_mutex.
//!
//! On server restart, we stop any running GC in the old process to avoid
//! having the exclusive lock held for too long.
//!
//! ## Locking table
//!
//! Below table shows all operations that play a role in locking, and which
//! mechanisms are used to make their concurrent usage safe.
//!
//! | starting ><br>v during | read index file | create index file | GC mark | GC sweep | update manifest | forget | prune | create backup | verify | reader api |
//! |-|-|-|-|-|-|-|-|-|-|-|
//! | **read index file** | / | / | / | / | / | mmap stays valid, oldest_shared_lock prevents GC | see forget column | / | / | / |
//! | **create index file** | / | / | / | / | / | / | / | /, happens at the end, after all chunks are touched | /, only happens without a manifest | / |
//! | **GC mark** | / | Datastore process-lock shared | gc_mutex, exclusive ProcessLocker | gc_mutex | /, GC only cares about index files, not manifests | tells GC about removed chunks | see forget column | /, index files don’t exist yet | / | / |
//! | **GC sweep** | / | Datastore process-lock shared | gc_mutex, exclusive ProcessLocker | gc_mutex | / | /, chunks already marked | see forget column | chunks get touched; chunk_store.mutex; oldest PL lock | / | / |
//! | **update manifest** | / | / | / | / | update_manifest lock | update_manifest lock, remove dir under lock | see forget column | /, “write manifest” happens at the end | /, can call “write manifest”, see that column | / |
//! | **forget** | / | / | removed_during_gc mutex is held during unlink | marking done, doesn’t matter if forgotten now | update_manifest lock, forget waits for lock | /, unlink is atomic | causes forget to fail, but that’s OK | running backup has snapshot flock | /, potentially detects missing folder | shared snap flock |
//! | **prune** | / | / | see forget row | see forget row | see forget row | causes warn in prune, but no error | see forget column | running and last non-running can’t be pruned | see forget row | shared snap flock |
//! | **create backup** | / | only time this happens, thus has snapshot flock | / | chunks get touched; chunk_store.mutex; oldest PL lock | no lock, but cannot exist beforehand | snapshot flock, can’t be forgotten | running and last non-running can’t be pruned | snapshot group flock, only one running per group | /, won’t be verified since manifest missing | / |
//! | **verify** | / | / | / | / | see “update manifest” row | /, potentially detects missing folder | see forget column | / | /, but useless (“update manifest” protects itself) | / |
//! | **reader api** | / | / | / | /, open snap can’t be forgotten, so ref must exist | / | prevented by shared snap flock | prevented by shared snap flock | / | / | /, lock is shared |!
//! * / = no interaction
//! * shared/exclusive from POV of 'starting' process

#![deny(unsafe_op_in_unsafe_fn)]

// Note: .pcat1 => Proxmox Catalog Format version 1
pub const CATALOG_NAME: &str = "catalog.pcat1.didx";

/// Directory path where active operations counters are saved.
pub const ACTIVE_OPERATIONS_DIR: &str = concat!(
    pbs_buildcfg::PROXMOX_BACKUP_RUN_DIR_M!(),
    "/active-operations"
);

#[macro_export]
macro_rules! PROXMOX_BACKUP_PROTOCOL_ID_V1 {
    () => {
        "proxmox-backup-protocol-v1"
    };
}

#[macro_export]
macro_rules! PROXMOX_BACKUP_READER_PROTOCOL_ID_V1 {
    () => {
        "proxmox-backup-reader-protocol-v1"
    };
}

pub mod backup_info;
pub mod cached_chunk_reader;
pub mod catalog;
pub mod checksum_reader;
pub mod checksum_writer;
pub mod chunk_stat;
pub mod chunk_store;
pub mod chunker;
pub mod crypt_reader;
pub mod crypt_writer;
pub mod data_blob;
pub mod data_blob_reader;
pub mod data_blob_writer;
pub mod file_formats;
pub mod index;
pub mod manifest;
pub mod paperkey;
pub mod prune;
pub mod read_chunk;
pub mod store_progress;
pub mod task_tracking;

pub mod dynamic_index;
pub mod fixed_index;

pub use backup_info::{BackupDir, BackupGroup, BackupInfo};
pub use checksum_reader::ChecksumReader;
pub use checksum_writer::ChecksumWriter;
pub use chunk_store::ChunkStore;
pub use chunker::Chunker;
pub use crypt_reader::CryptReader;
pub use crypt_writer::CryptWriter;
pub use data_blob::DataBlob;
pub use data_blob_reader::DataBlobReader;
pub use data_blob_writer::DataBlobWriter;
pub use manifest::BackupManifest;
pub use store_progress::StoreProgress;

mod datastore;
pub use datastore::{check_backup_owner, DataStore};

mod hierarchy;
pub use hierarchy::{
    ListGroups, ListGroupsType, ListNamespaces, ListNamespacesRecursive, ListSnapshots,
};

mod snapshot_reader;
pub use snapshot_reader::SnapshotReader;

mod local_chunk_reader;
pub use local_chunk_reader::LocalChunkReader;
