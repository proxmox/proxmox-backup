//! Magnetic tape backup

use anyhow::{format_err, Error};

use proxmox_sys::fs::{create_path, CreateOptions};

use pbs_buildcfg::{PROXMOX_BACKUP_RUN_DIR_M, PROXMOX_BACKUP_STATE_DIR_M};

#[cfg(test)]
mod test;

pub mod file_formats;

mod media_set;
pub use media_set::*;

mod inventory;
pub use inventory::*;

pub mod changer;
pub mod drive;
pub mod encryption_keys;

mod media_pool;
pub use media_pool::*;

mod media_catalog;
pub use media_catalog::*;

mod media_catalog_cache;
pub use media_catalog_cache::*;

mod pool_writer;
pub use pool_writer::*;

/// Directory path where we store all tape status information
pub const TAPE_STATUS_DIR: &str = concat!(PROXMOX_BACKUP_STATE_DIR_M!(), "/tape");

/// Directory path where we store drive lock file
pub const DRIVE_LOCK_DIR: &str = concat!(PROXMOX_BACKUP_RUN_DIR_M!(), "/drive-lock");

/// Directory path where we store temporary drive state
pub const DRIVE_STATE_DIR: &str = concat!(PROXMOX_BACKUP_RUN_DIR_M!(), "/drive-state");

/// Directory path where we store cached changer state
pub const CHANGER_STATE_DIR: &str = concat!(PROXMOX_BACKUP_RUN_DIR_M!(), "/changer-state");

/// We limit chunk archive size, so that we can faster restore a
/// specific chunk (The catalog only store file numbers, so we
/// need to read the whole archive to restore a single chunk)
pub const MAX_CHUNK_ARCHIVE_SIZE: usize = 4 * 1024 * 1024 * 1024; // 4GB for now

/// To improve performance, we need to avoid tape drive buffer flush.
pub const COMMIT_BLOCK_SIZE: usize = 128 * 1024 * 1024 * 1024; // 128 GiB

/// Create tape status dir with correct permission
pub fn create_tape_status_dir() -> Result<(), Error> {
    let backup_user = pbs_config::backup_user()?;
    let mode = nix::sys::stat::Mode::from_bits_truncate(0o0750);
    let options = CreateOptions::new()
        .perm(mode)
        .owner(backup_user.uid)
        .group(backup_user.gid);

    let parent_opts = CreateOptions::new()
        .owner(backup_user.uid)
        .group(backup_user.gid);

    create_path(TAPE_STATUS_DIR, Some(parent_opts), Some(options))
        .map_err(|err: Error| format_err!("unable to create tape status dir - {}", err))?;

    Ok(())
}

/// Create drive lock dir with correct permission
pub fn create_drive_lock_dir() -> Result<(), Error> {
    let backup_user = pbs_config::backup_user()?;
    let mode = nix::sys::stat::Mode::from_bits_truncate(0o0750);
    let options = CreateOptions::new()
        .perm(mode)
        .owner(backup_user.uid)
        .group(backup_user.gid);

    let parent_opts = CreateOptions::new()
        .owner(backup_user.uid)
        .group(backup_user.gid);

    create_path(DRIVE_LOCK_DIR, Some(parent_opts), Some(options))
        .map_err(|err: Error| format_err!("unable to create drive state dir - {}", err))?;

    Ok(())
}

/// Create drive state dir with correct permission
pub fn create_drive_state_dir() -> Result<(), Error> {
    let backup_user = pbs_config::backup_user()?;
    let mode = nix::sys::stat::Mode::from_bits_truncate(0o0750);
    let options = CreateOptions::new()
        .perm(mode)
        .owner(backup_user.uid)
        .group(backup_user.gid);

    let parent_opts = CreateOptions::new()
        .owner(backup_user.uid)
        .group(backup_user.gid);

    create_path(DRIVE_STATE_DIR, Some(parent_opts), Some(options))
        .map_err(|err: Error| format_err!("unable to create drive state dir - {}", err))?;

    Ok(())
}

/// Create changer state cache dir with correct permission
pub fn create_changer_state_dir() -> Result<(), Error> {
    let backup_user = pbs_config::backup_user()?;
    let mode = nix::sys::stat::Mode::from_bits_truncate(0o0750);
    let options = CreateOptions::new()
        .perm(mode)
        .owner(backup_user.uid)
        .group(backup_user.gid);

    let parent_opts = CreateOptions::new()
        .owner(backup_user.uid)
        .group(backup_user.gid);

    create_path(CHANGER_STATE_DIR, Some(parent_opts), Some(options))
        .map_err(|err: Error| format_err!("unable to create changer state dir - {}", err))?;

    Ok(())
}
