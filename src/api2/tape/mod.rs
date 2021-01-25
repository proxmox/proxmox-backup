//! Tape Backup Management

use proxmox::api::router::SubdirMap;
use proxmox::api::Router;
use proxmox::list_subdirs_api_method;

pub mod drive;
pub mod changer;
pub mod media;
pub mod backup;
pub mod restore;

pub const SUBDIRS: SubdirMap = &[
    ("backup", &backup::ROUTER),
    ("changer", &changer::ROUTER),
    ("drive", &drive::ROUTER),
    ("media", &media::ROUTER),
    ("restore", &restore::ROUTER),
    (
        "scan-changers",
        &Router::new()
            .get(&changer::API_METHOD_SCAN_CHANGERS),
    ),
];

pub const ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(SUBDIRS))
    .subdirs(SUBDIRS);
