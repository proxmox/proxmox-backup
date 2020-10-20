use proxmox::api::router::{Router, SubdirMap};
use proxmox::list_subdirs_api_method;

pub mod datastore;
pub mod remote;
pub mod sync;
pub mod verify;

const SUBDIRS: SubdirMap = &[
    ("datastore", &datastore::ROUTER),
    ("remote", &remote::ROUTER),
    ("sync", &sync::ROUTER),
    ("verify", &verify::ROUTER)
];

pub const ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(SUBDIRS))
    .subdirs(SUBDIRS);
