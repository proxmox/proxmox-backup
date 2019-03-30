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
//! implies that we need some kink of locking for the
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
//!   Note: We create temporary (.tmp) file, then do an atomic rename ...
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
//! # Garbage Collection
//!
//! Deleting backups is as easy as deleting the corresponding .idx
//! files. Unfortunately, this does not free up any storage, because
//! those files just contains references to chunks.
//!
//! To free up some storage, we run a garbage collection process at
//! regular intervals. The collector uses an mark and sweep
//! approach. In the first run, it scans all .idx files to mark used
//! chunks. The second run then removes all unmarked chunks from the
//! store.
//!
//! The above locking mechanism makes sure that we are the only
//! process running GC.
//!
//!


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
