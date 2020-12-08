use anyhow::{bail, Error};
use serde_json::Value;

use proxmox::api::{api, Router, SubdirMap};
use proxmox::list_subdirs_api_method;

use crate::{
    config,
    api2::types::{
        DRIVE_ID_SCHEMA,
        LinuxTapeDrive,
        ScsiTapeChanger,
        TapeDeviceInfo,
    },
    tape::{
        MediaChange,
        mtx_load,
        mtx_unload,
        linux_tape_device_list,
    },
};

#[api(
    input: {
        properties: {
            name: {
                schema: DRIVE_ID_SCHEMA,
            },
            slot: {
                description: "Source slot number",
                minimum: 1,
            },
        },
    },
)]
/// Load media via changer from slot
pub fn load_slot(
    name: String,
    slot: u64,
    _param: Value,
) -> Result<(), Error> {

    let (config, _digest) = config::drive::config()?;

    let drive: LinuxTapeDrive = config.lookup("linux", &name)?;

    let changer: ScsiTapeChanger = match drive.changer {
        Some(ref changer) => config.lookup("changer", changer)?,
        None => bail!("drive '{}' has no associated changer", name),
    };

    let drivenum = 0;

    mtx_load(&changer.path, slot, drivenum)
}

#[api(
    input: {
        properties: {
            name: {
                schema: DRIVE_ID_SCHEMA,
            },
            slot: {
                description: "Target slot number. If omitted, defaults to the slot that the drive was loaded from.",
                minimum: 1,
                optional: true,
            },
        },
    },
)]
/// Unload media via changer
pub fn unload(
    name: String,
    slot: Option<u64>,
    _param: Value,
) -> Result<(), Error> {

    let (config, _digest) = config::drive::config()?;

    let mut drive: LinuxTapeDrive = config.lookup("linux", &name)?;

    let changer: ScsiTapeChanger = match drive.changer {
        Some(ref changer) => config.lookup("changer", changer)?,
        None => bail!("drive '{}' has no associated changer", name),
    };

    let drivenum: u64 = 0;

    if let Some(slot) = slot {
        mtx_unload(&changer.path, slot, drivenum)
    } else {
        drive.unload_media()
    }
}

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

    let list = linux_tape_device_list();

    Ok(list)
}

pub const SUBDIRS: SubdirMap = &[
    (
        "load-slot",
        &Router::new()
            .put(&API_METHOD_LOAD_SLOT)
    ),
    (
        "scan",
        &Router::new()
            .get(&API_METHOD_SCAN_DRIVES)
    ),
    (
        "unload",
        &Router::new()
            .put(&API_METHOD_UNLOAD)
    ),
];

pub const ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(SUBDIRS))
    .subdirs(SUBDIRS);
