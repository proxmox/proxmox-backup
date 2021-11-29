use proxmox_router::{Router, SubdirMap};
use proxmox_router::list_subdirs_api_method;
use proxmox_sys::sortable;

pub mod tfa;
pub mod openid;

#[sortable]
const SUBDIRS: SubdirMap = &sorted!([
    ("openid", &openid::ROUTER),
    ("tfa", &tfa::ROUTER),
]);

pub const ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(SUBDIRS))
    .subdirs(SUBDIRS);
