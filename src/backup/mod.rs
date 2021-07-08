//! Server/client-specific parts for what's otherwise in pbs-datastore.

use anyhow::{bail, Error};

// Note: .pcat1 => Proxmox Catalog Format version 1
pub const CATALOG_NAME: &str = "catalog.pcat1.didx";

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

pub use pbs_datastore::dynamic_index::*;
pub use pbs_datastore::fixed_index;
pub use pbs_datastore::fixed_index::*;

pub use pbs_datastore::read_chunk::*;

mod chunk_stream;
pub use chunk_stream::*;

// Split
mod read_chunk;
pub use read_chunk::*;

// Split
mod dynamic_index;
pub use dynamic_index::*;

mod datastore;
pub use datastore::*;

mod store_progress;
pub use store_progress::*;

mod verify;
pub use verify::*;

// Move to client
mod catalog_shell;
pub use catalog_shell::*;

mod cached_chunk_reader;
pub use cached_chunk_reader::*;
