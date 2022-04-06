//! Tape Backup Management

use anyhow::Error;
use serde_json::Value;

use proxmox_router::{list_subdirs_api_method, Router, SubdirMap};
use proxmox_schema::api;

use pbs_api_types::TapeDeviceInfo;
use pbs_tape::linux_list_drives::{linux_tape_changer_list, lto_tape_device_list};

pub mod backup;
pub mod changer;
pub mod drive;
pub mod media;
pub mod restore;

#[api(
    input: {
        properties: {},
    },
    returns: {
        description: "The list of autodetected tape drives.",
        type: Array,
        items: {
            type: TapeDeviceInfo,
        },
    },
)]
/// Scan tape drives
pub fn scan_drives(_param: Value) -> Result<Vec<TapeDeviceInfo>, Error> {
    let list = lto_tape_device_list();

    Ok(list)
}

#[api(
    input: {
        properties: {},
    },
    returns: {
        description: "The list of autodetected tape changers.",
        type: Array,
        items: {
            type: TapeDeviceInfo,
        },
    },
)]
/// Scan for SCSI tape changers
pub fn scan_changers(_param: Value) -> Result<Vec<TapeDeviceInfo>, Error> {
    let list = linux_tape_changer_list();

    Ok(list)
}

const SUBDIRS: SubdirMap = &[
    ("backup", &backup::ROUTER),
    ("changer", &changer::ROUTER),
    ("drive", &drive::ROUTER),
    ("media", &media::ROUTER),
    ("restore", &restore::ROUTER),
    (
        "scan-changers",
        &Router::new().get(&API_METHOD_SCAN_CHANGERS),
    ),
    ("scan-drives", &Router::new().get(&API_METHOD_SCAN_DRIVES)),
];

pub const ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(SUBDIRS))
    .subdirs(SUBDIRS);
