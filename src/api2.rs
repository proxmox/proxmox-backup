pub mod types;
pub mod config;
pub mod admin;
pub mod backup;
pub mod reader;
pub mod node;
pub mod version;
mod subscription;
mod access;

use crate::api_schema::router::*;

const NODES_ROUTER: Router = Router::new()
    .match_all("node", &node::ROUTER);

pub const SUBDIRS: SubdirMap = &[
    ("access", &access::ROUTER),
    ("admin", &admin::ROUTER),
    ("backup", &backup::ROUTER),
    ("config", &config::ROUTER),
    ("nodes", &NODES_ROUTER),
    ("reader", &reader::ROUTER),
    ("subscription", &subscription::ROUTER),
    ("version", &version::ROUTER),
];

pub const ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(SUBDIRS))
    .subdirs(SUBDIRS);
