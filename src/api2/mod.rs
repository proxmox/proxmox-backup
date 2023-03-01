//! The Proxmox Backup Server API

use proxmox_sortable_macro::sortable;

pub mod access;
pub mod admin;
pub mod backup;
pub mod config;
pub mod helpers;
pub mod node;
pub mod ping;
pub mod pull;
pub mod reader;
pub mod status;
pub mod tape;
pub mod types;
pub mod version;

use proxmox_router::{list_subdirs_api_method, Router, SubdirMap};

#[sortable]
const SUBDIRS: SubdirMap = &sorted!([
    ("access", &access::ROUTER),
    ("admin", &admin::ROUTER),
    ("backup", &backup::ROUTER),
    ("config", &config::ROUTER),
    ("nodes", &node::ROUTER),
    ("ping", &ping::ROUTER),
    ("pull", &pull::ROUTER),
    ("reader", &reader::ROUTER),
    ("status", &status::ROUTER),
    ("tape", &tape::ROUTER),
    ("version", &version::ROUTER),
]);

pub const ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(SUBDIRS))
    .subdirs(SUBDIRS);
