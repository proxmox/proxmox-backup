pub mod file_formats;

mod tape_write;
pub use tape_write::*;

mod tape_read;
pub use tape_read::*;

mod inventory;
pub use inventory::*;

mod changer;
pub use changer::*;

/// Directory path where we stora all status information
pub const MEDIA_POOL_STATUS_DIR: &str = "/var/lib/proxmox-backup/mediapool";

/// We limit chunk archive size, so that we can faster restore a
/// specific chunk (The catalog only store file numbers, so we
/// need to read the whole archive to restore a single chunk)
pub const MAX_CHUNK_ARCHIVE_SIZE: usize = 4*1024*1024*1024; // 4GB for now

/// To improve performance, we need to avoid tape drive buffer flush.
pub const COMMIT_BLOCK_SIZE: usize = 128*1024*1024*1024; // 128 GiB
