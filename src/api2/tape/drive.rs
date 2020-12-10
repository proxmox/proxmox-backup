use anyhow::{bail, Error};
use serde_json::Value;

use proxmox::api::{api, Router, SubdirMap};
use proxmox::list_subdirs_api_method;

use crate::{
    config,
    api2::types::{
        DRIVE_ID_SCHEMA,
        MEDIA_LABEL_SCHEMA,
        LinuxTapeDrive,
        ScsiTapeChanger,
        TapeDeviceInfo,
    },
    tape::{
        MediaChange,
        mtx_load,
        mtx_unload,
        linux_tape_device_list,
        open_drive,
        media_changer,
    },
};

#[api(
    input: {
        properties: {
            drive: {
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
    drive: String,
    slot: u64,
    _param: Value,
) -> Result<(), Error> {

    let (config, _digest) = config::drive::config()?;

    let drive_config: LinuxTapeDrive = config.lookup("linux", &drive)?;

    let changer: ScsiTapeChanger = match drive_config.changer {
        Some(ref changer) => config.lookup("changer", changer)?,
        None => bail!("drive '{}' has no associated changer", drive),
    };

    let drivenum = 0;

    mtx_load(&changer.path, slot, drivenum)
}

#[api(
    input: {
        properties: {
            drive: {
                schema: DRIVE_ID_SCHEMA,
            },
            "changer-id": {
                schema: MEDIA_LABEL_SCHEMA,
            },
        },
    },
)]
/// Load media with specified label
///
/// Issue a media load request to the associated changer device.
pub fn load_media(drive: String, changer_id: String) -> Result<(), Error> {

    let (config, _digest) = config::drive::config()?;

    let (mut changer, _) = media_changer(&config, &drive, false)?;

    changer.load_media(&changer_id)?;

    Ok(())
}

#[api(
    input: {
        properties: {
            drive: {
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
    drive: String,
    slot: Option<u64>,
    _param: Value,
) -> Result<(), Error> {

    let (config, _digest) = config::drive::config()?;

    let mut drive_config: LinuxTapeDrive = config.lookup("linux", &drive)?;

    let changer: ScsiTapeChanger = match drive_config.changer {
        Some(ref changer) => config.lookup("changer", changer)?,
        None => bail!("drive '{}' has no associated changer", drive),
    };

    let drivenum: u64 = 0;

    if let Some(slot) = slot {
        mtx_unload(&changer.path, slot, drivenum)
    } else {
        drive_config.unload_media()
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

#[api(
    input: {
        properties: {
            drive: {
                schema: DRIVE_ID_SCHEMA,
            },
            fast: {
                description: "Use fast erase.",
                type: bool,
                optional: true,
                default: true,
            },
        },
    },
)]
/// Erase media
pub fn erase_media(drive: String, fast: Option<bool>) -> Result<(), Error> {

    let (config, _digest) = config::drive::config()?;

    let mut drive = open_drive(&config, &drive)?;

    drive.erase_media(fast.unwrap_or(true))?;

    Ok(())
}

#[api(
    input: {
        properties: {
            drive: {
                schema: DRIVE_ID_SCHEMA,
            },
        },
    },
)]
/// Rewind tape
pub fn rewind(drive: String) -> Result<(), Error> {

    let (config, _digest) = config::drive::config()?;

    let mut drive = open_drive(&config, &drive)?;

    drive.rewind()?;

    Ok(())
}

#[api(
    input: {
        properties: {
            drive: {
                schema: DRIVE_ID_SCHEMA,
            },
        },
    },
)]
/// Eject/Unload drive media
pub fn eject_media(drive: String) -> Result<(), Error> {

    let (config, _digest) = config::drive::config()?;

    let (mut changer, _) = media_changer(&config, &drive, false)?;

    if !changer.eject_on_unload() {
        let mut drive = open_drive(&config, &drive)?;
        drive.eject_media()?;
    }

    changer.unload_media()?;

    Ok(())
}

pub const SUBDIRS: SubdirMap = &[
    (
        "rewind",
        &Router::new()
            .put(&API_METHOD_REWIND)
    ),
    (
        "erase-media",
        &Router::new()
            .put(&API_METHOD_ERASE_MEDIA)
    ),
    (
        "eject-media",
        &Router::new()
            .put(&API_METHOD_EJECT_MEDIA)
    ),
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
