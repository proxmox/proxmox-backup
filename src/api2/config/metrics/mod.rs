use proxmox_router::list_subdirs_api_method;
use proxmox_router::{Router, SubdirMap};
use proxmox_sortable_macro::sortable;

pub mod influxdbhttp;
pub mod influxdbudp;

#[sortable]
const SUBDIRS: SubdirMap = &sorted!([
    ("influxdb-http", &influxdbhttp::ROUTER),
    ("influxdb-udp", &influxdbudp::ROUTER),
]);

pub const ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(SUBDIRS))
    .subdirs(SUBDIRS);
