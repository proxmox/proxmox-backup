use proxmox::api::router::{Router, SubdirMap};
use proxmox::api::list_subdirs_api_method;

pub mod datastore;
pub mod remotes;

const SUBDIRS: SubdirMap = &[
    ("datastore", &datastore::ROUTER),
    ("remotes", &remotes::ROUTER),
];

pub const ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(SUBDIRS))
    .subdirs(SUBDIRS);
