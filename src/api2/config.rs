use proxmox::api::router::{Router, SubdirMap};
use proxmox::list_subdirs_api_method;

pub mod datastore;
pub mod remote;
pub mod job;

const SUBDIRS: SubdirMap = &[
    ("datastore", &datastore::ROUTER),
    ("job", &job::ROUTER),
    ("remote", &remote::ROUTER),
];

pub const ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(SUBDIRS))
    .subdirs(SUBDIRS);
