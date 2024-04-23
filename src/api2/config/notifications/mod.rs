use proxmox_router::list_subdirs_api_method;
use proxmox_router::{Router, SubdirMap};
use proxmox_sortable_macro::sortable;

mod matchers;
mod sendmail;
mod targets;

#[sortable]
const SUBDIRS: SubdirMap = &sorted!([
    ("endpoints", &ENDPOINT_ROUTER),
    ("targets", &targets::ROUTER),
    ("matchers", &matchers::ROUTER),
]);

pub const ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(SUBDIRS))
    .subdirs(SUBDIRS);

#[sortable]
const ENDPOINT_SUBDIRS: SubdirMap = &sorted!([("sendmail", &sendmail::ROUTER),]);

const ENDPOINT_ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(ENDPOINT_SUBDIRS))
    .subdirs(ENDPOINT_SUBDIRS);
