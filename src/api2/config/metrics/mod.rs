use proxmox_router::{Router, SubdirMap};
use proxmox_router::list_subdirs_api_method;
use proxmox_sys::sortable;

pub mod influxdbudp;
pub mod influxdbhttp;

#[sortable]
const SUBDIRS: SubdirMap = &sorted!([
    ("influxdb-http", &influxdbhttp::ROUTER),
    ("influxdb-udp", &influxdbudp::ROUTER),
]);

pub const ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(SUBDIRS))
    .subdirs(SUBDIRS);
