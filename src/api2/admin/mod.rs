//! Backup Server Administration

use proxmox_router::list_subdirs_api_method;
use proxmox_router::{Router, SubdirMap};
use proxmox_sortable_macro::sortable;

pub mod datastore;
pub mod metrics;
pub mod namespace;
pub mod prune;
pub mod sync;
pub mod traffic_control;
pub mod verify;

#[sortable]
const SUBDIRS: SubdirMap = &sorted!([
    ("datastore", &datastore::ROUTER),
    ("metrics", &metrics::ROUTER),
    ("prune", &prune::ROUTER),
    ("sync", &sync::ROUTER),
    ("traffic-control", &traffic_control::ROUTER),
    ("verify", &verify::ROUTER),
]);

pub const ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(SUBDIRS))
    .subdirs(SUBDIRS);
