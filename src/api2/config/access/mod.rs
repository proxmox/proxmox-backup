use proxmox_router::list_subdirs_api_method;
use proxmox_router::{Router, SubdirMap};
use proxmox_sortable_macro::sortable;

pub mod ldap;
pub mod openid;
pub mod tfa;

#[sortable]
const SUBDIRS: SubdirMap = &sorted!([
    ("ldap", &ldap::ROUTER),
    ("openid", &openid::ROUTER),
    ("tfa", &tfa::ROUTER),
]);

pub const ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(SUBDIRS))
    .subdirs(SUBDIRS);
