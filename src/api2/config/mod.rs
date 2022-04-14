//! Backup Server Configuration

use proxmox_router::list_subdirs_api_method;
use proxmox_router::{Router, SubdirMap};

pub mod access;
pub mod acme;
pub mod changer;
pub mod datastore;
pub mod drive;
pub mod media_pool;
pub mod remote;
pub mod sync;
pub mod tape_backup_job;
pub mod tape_encryption_keys;
pub mod traffic_control;
pub mod verify;

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
