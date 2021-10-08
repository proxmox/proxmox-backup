//! Backup Server Administration

use proxmox_router::{Router, SubdirMap};
use proxmox_router::list_subdirs_api_method;

pub mod datastore;
pub mod sync;
pub mod verify;

const SUBDIRS: SubdirMap = &[
    ("datastore", &datastore::ROUTER),
    ("sync", &sync::ROUTER),
    ("verify", &verify::ROUTER)
];

pub const ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(SUBDIRS))
    .subdirs(SUBDIRS);
