use proxmox::api::router::{Router, SubdirMap};
use proxmox::list_subdirs_api_method;

pub mod datastore;
pub mod remote;
pub mod sync;
pub mod verify;
pub mod drive;
pub mod changer;

const SUBDIRS: SubdirMap = &[
    ("changer", &changer::ROUTER),
    ("datastore", &datastore::ROUTER),
    ("drive", &drive::ROUTER),
    ("remote", &remote::ROUTER),
    ("scan-changers", &changer::SCAN_CHANGERS),
    ("scan-drives", &drive::SCAN_DRIVES),
    ("sync", &sync::ROUTER),
    ("verify", &verify::ROUTER)
];

pub const ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(SUBDIRS))
    .subdirs(SUBDIRS);
