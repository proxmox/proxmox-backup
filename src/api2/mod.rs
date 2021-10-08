//! The Proxmox Backup Server API

pub mod access;
pub mod admin;
pub mod backup;
pub mod config;
pub mod node;
pub mod reader;
pub mod status;
pub mod types;
pub mod version;
pub mod ping;
pub mod pull;
pub mod tape;
pub mod helpers;

use proxmox_router::{list_subdirs_api_method, Router, SubdirMap};

const SUBDIRS: SubdirMap = &[
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
];

pub const ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(SUBDIRS))
    .subdirs(SUBDIRS);
