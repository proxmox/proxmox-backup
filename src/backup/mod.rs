//! Server/client-specific parts for what's otherwise in pbs-datastore.

use anyhow::{bail, Error};

// Note: .pcat1 => Proxmox Catalog Format version 1
pub const CATALOG_NAME: &str = "catalog.pcat1.didx";

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

pub use pbs_datastore::backup_info;
pub use pbs_datastore::backup_info::*;
pub use pbs_datastore::catalog;
pub use pbs_datastore::catalog::*;
pub use pbs_datastore::checksum_reader;
pub use pbs_datastore::checksum_reader::*;
pub use pbs_datastore::checksum_writer;
pub use pbs_datastore::checksum_writer::*;
pub use pbs_datastore::chunk_stat;
pub use pbs_datastore::chunk_stat::*;
pub use pbs_datastore::chunk_store;
pub use pbs_datastore::chunk_store::*;
pub use pbs_datastore::chunker;
pub use pbs_datastore::chunker::*;
pub use pbs_datastore::crypt_config;
pub use pbs_datastore::crypt_config::*;
pub use pbs_datastore::crypt_reader;
pub use pbs_datastore::crypt_reader::*;
pub use pbs_datastore::crypt_writer;
pub use pbs_datastore::crypt_writer::*;
pub use pbs_datastore::data_blob;
pub use pbs_datastore::data_blob::*;
pub use pbs_datastore::data_blob_reader;
pub use pbs_datastore::data_blob_reader::*;
pub use pbs_datastore::data_blob_writer;
pub use pbs_datastore::data_blob_writer::*;
pub use pbs_datastore::file_formats;
pub use pbs_datastore::file_formats::*;
pub use pbs_datastore::index;
pub use pbs_datastore::index::*;
pub use pbs_datastore::key_derivation;
pub use pbs_datastore::key_derivation::*;
pub use pbs_datastore::manifest;
pub use pbs_datastore::manifest::*;
pub use pbs_datastore::prune;
pub use pbs_datastore::prune::*;

pub use pbs_datastore::store_progress::StoreProgress;

pub use pbs_datastore::dynamic_index::*;
pub use pbs_datastore::fixed_index;
pub use pbs_datastore::fixed_index::*;

pub use pbs_datastore::read_chunk::*;

// Split
mod read_chunk;
pub use read_chunk::*;

// Split
mod dynamic_index;
pub use dynamic_index::*;

mod datastore;
pub use datastore::*;

mod verify;
pub use verify::*;

mod cached_chunk_reader;
pub use cached_chunk_reader::*;

pub struct BackupLockGuard(std::fs::File);

/// Open or create a lock file owned by user "backup" and lock it.
///
/// Owner/Group of the file is set to backup/backup.
/// File mode is 0660.
/// Default timeout is 10 seconds.
///
/// Note: This method needs to be called by user "root" or "backup".
pub fn open_backup_lockfile<P: AsRef<std::path::Path>>(
    path: P,
    timeout: Option<std::time::Duration>,
    exclusive: bool,
) -> Result<BackupLockGuard, Error> {
    let user = backup_user()?;
    let options = proxmox::tools::fs::CreateOptions::new()
        .perm(nix::sys::stat::Mode::from_bits_truncate(0o660))
        .owner(user.uid)
        .group(user.gid);

    let timeout = timeout.unwrap_or(std::time::Duration::new(10, 0));

    let file = proxmox::tools::fs::open_file_locked(&path, timeout, exclusive, options)?;
    Ok(BackupLockGuard(file))
}

/// Atomically write data to file owned by "root:backup" with permission "0640"
///
/// Only the superuser can write those files, but group 'backup' can read them.
pub fn replace_backup_config<P: AsRef<std::path::Path>>(
    path: P,
    data: &[u8],
) -> Result<(), Error> {
    let backup_user = backup_user()?;
    let mode = nix::sys::stat::Mode::from_bits_truncate(0o0640);
    // set the correct owner/group/permissions while saving file
    // owner(rw) = root, group(r)= backup
    let options = proxmox::tools::fs::CreateOptions::new()
        .perm(mode)
        .owner(nix::unistd::ROOT)
        .group(backup_user.gid);

    proxmox::tools::fs::replace_file(path, data, options)?;

    Ok(())
}

/// Atomically write data to file owned by "root:root" with permission "0600"
///
/// Only the superuser can read and write those files.
pub fn replace_secret_config<P: AsRef<std::path::Path>>(
    path: P,
    data: &[u8],
) -> Result<(), Error> {
    let mode = nix::sys::stat::Mode::from_bits_truncate(0o0600);
    // set the correct owner/group/permissions while saving file
    // owner(rw) = root, group(r)= root
    let options = proxmox::tools::fs::CreateOptions::new()
        .perm(mode)
        .owner(nix::unistd::ROOT)
        .group(nix::unistd::Gid::from_raw(0));

    proxmox::tools::fs::replace_file(path, data, options)?;

    Ok(())
}
