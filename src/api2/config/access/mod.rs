use proxmox::api::{Router, SubdirMap};
use proxmox::list_subdirs_api_method;

pub mod tfa;

const SUBDIRS: SubdirMap = &[("tfa", &tfa::ROUTER)];

pub const ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(SUBDIRS))
    .subdirs(SUBDIRS);
