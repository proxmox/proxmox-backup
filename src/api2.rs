pub mod access;
pub mod admin;
pub mod backup;
pub mod config;
pub mod node;
pub mod reader;
mod subscription;
pub mod status;
pub mod types;
pub mod version;
pub mod pull;
mod helpers;

use proxmox::api::router::SubdirMap;
use proxmox::api::Router;
use proxmox::list_subdirs_api_method;

const NODES_ROUTER: Router = Router::new().match_all("node", &node::ROUTER);

pub const SUBDIRS: SubdirMap = &[
    ("access", &access::ROUTER),
    ("admin", &admin::ROUTER),
    ("backup", &backup::ROUTER),
    ("config", &config::ROUTER),
    ("nodes", &NODES_ROUTER),
    ("pull", &pull::ROUTER),
    ("reader", &reader::ROUTER),
    ("status", &status::ROUTER),
    ("subscription", &subscription::ROUTER),
    ("version", &version::ROUTER),
];

pub const ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(SUBDIRS))
    .subdirs(SUBDIRS);
