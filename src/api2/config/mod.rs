//! Backup Server Configuration

use proxmox_router::{Router, SubdirMap};
use proxmox_router::list_subdirs_api_method;

pub mod access;
pub mod acme;
pub mod datastore;
pub mod remote;
pub mod sync;
pub mod verify;
pub mod drive;
pub mod changer;
pub mod media_pool;
pub mod tape_encryption_keys;
pub mod tape_backup_job;
pub mod traffic_control;

const SUBDIRS: SubdirMap = &[
    ("access", &access::ROUTER),
    ("acme", &acme::ROUTER),
    ("changer", &changer::ROUTER),
    ("datastore", &datastore::ROUTER),
    ("drive", &drive::ROUTER),
    ("media-pool", &media_pool::ROUTER),
    ("remote", &remote::ROUTER),
    ("sync", &sync::ROUTER),
    ("tape-backup-job", &tape_backup_job::ROUTER),
    ("tape-encryption-keys", &tape_encryption_keys::ROUTER),
    ("traffic-control", &traffic_control::ROUTER),
    ("verify", &verify::ROUTER),
];

pub const ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(SUBDIRS))
    .subdirs(SUBDIRS);
