use anyhow::{bail, Error};
use serde_json::Value;

use proxmox::api::{api, Router, RpcEnvironment};

use crate::{
    config,
    api2::types::{
        DRIVE_ID_SCHEMA,
        CHANGER_ID_SCHEMA,
        LINUX_DRIVE_PATH_SCHEMA,
        DriveListEntry,
        LinuxTapeDrive,
        ScsiTapeChanger,
        TapeDeviceInfo,
    },
    tape::{
        linux_tape_device_list,
        check_drive_path,
        lookup_drive,
    },
};

#[api(
    input: {
        properties: {
            name: {
                schema: DRIVE_ID_SCHEMA,
            },
            path: {
                schema: LINUX_DRIVE_PATH_SCHEMA,
            },
        },
    },
)]
/// Create a new drive
pub fn create_drive(param: Value) -> Result<(), Error> {

    let _lock = config::drive::lock()?;

    let (mut config, _digest) = config::drive::config()?;

    let item: LinuxTapeDrive = serde_json::from_value(param)?;

    let linux_drives = linux_tape_device_list();

    check_drive_path(&linux_drives, &item.path)?;

    if config.sections.get(&item.name).is_some() {
        bail!("Entry '{}' already exists", item.name);
    }

    config.set_data(&item.name, "linux", &item)?;

    config::drive::save_config(&config)?;

    Ok(())
}

#[api(
    input: {
        properties: {
            name: {
                schema: DRIVE_ID_SCHEMA,
            },
        },
    },
    returns: {
        type: LinuxTapeDrive,
    },
)]
/// Get drive configuration
pub fn get_config(
    name: String,
    _param: Value,
    mut rpcenv: &mut dyn RpcEnvironment,
) -> Result<LinuxTapeDrive, Error> {

    let (config, digest) = config::drive::config()?;

    let data: LinuxTapeDrive = config.lookup("linux", &name)?;

    rpcenv["digest"] = proxmox::tools::digest_to_hex(&digest).into();

    Ok(data)
}

#[api(
    input: {
        properties: {},
    },
    returns: {
        description: "The list of configured remotes (with config digest).",
        type: Array,
        items: {
            type: DriveListEntry,
        },
    },
)]
/// List drives
pub fn list_drives(
    _param: Value,
    mut rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<DriveListEntry>, Error> {

    let (config, digest) = config::drive::config()?;

    let linux_drives = linux_tape_device_list();

    let drive_list: Vec<LinuxTapeDrive> = config.convert_to_typed_array("linux")?;

    let mut list = Vec::new();

    for drive in drive_list {
        let mut entry = DriveListEntry {
            name: drive.name,
            path: drive.path.clone(),
            changer: drive.changer,
            vendor: None,
            model: None,
            serial: None,
        };
        if let Some(info) = lookup_drive(&linux_drives, &drive.path) {
            entry.vendor = Some(info.vendor.clone());
            entry.model = Some(info.model.clone());
            entry.serial = Some(info.serial.clone());
        }

        list.push(entry);
    }

    rpcenv["digest"] = proxmox::tools::digest_to_hex(&digest).into();
    Ok(list)
}

#[api(
    input: {
        properties: {
            name: {
                schema: DRIVE_ID_SCHEMA,
            },
            path: {
                schema: LINUX_DRIVE_PATH_SCHEMA,
                optional: true,
            },
            changer: {
                schema: CHANGER_ID_SCHEMA,
                optional: true,
            },
        },
    },
)]
/// Update a drive configuration
pub fn update_drive(
    name: String,
    path: Option<String>,
    changer: Option<String>,
    _param: Value,
) -> Result<(), Error> {

    let _lock = config::drive::lock()?;

    let (mut config, _digest) = config::drive::config()?;

    let mut data: LinuxTapeDrive = config.lookup("linux", &name)?;

    if let Some(path) = path {
        let linux_drives = linux_tape_device_list();
        check_drive_path(&linux_drives, &path)?;
        data.path = path;
    }

    if let Some(changer) = changer {
        let _: ScsiTapeChanger = config.lookup("changer", &changer)?;
        data.changer = Some(changer);
    }

    config.set_data(&name, "linux", &data)?;

    config::drive::save_config(&config)?;

    Ok(())
}

#[api(
    input: {
        properties: {
            name: {
                schema: DRIVE_ID_SCHEMA,
            },
        },
    },
)]
/// Delete a drive configuration
pub fn delete_drive(name: String, _param: Value) -> Result<(), Error> {

    let _lock = config::drive::lock()?;

    let (mut config, _digest) = config::drive::config()?;

    match config.sections.get(&name) {
        Some((section_type, _)) => {
            if section_type != "linux" {
                bail!("Entry '{}' exists, but is not a linux tape drive", name);
            }
            config.sections.remove(&name);
        },
        None => bail!("Delete drive '{}' failed - no such drive", name),
    }

    config::drive::save_config(&config)?;

    Ok(())
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
pub fn scan_tape_drives(_param: Value) -> Result<Vec<TapeDeviceInfo>, Error> {

    let list = linux_tape_device_list();

    Ok(list)
}


const SCAN_DRIVES: Router = Router::new()
    .get(&API_METHOD_SCAN_TAPE_DRIVES);


const ITEM_ROUTER: Router = Router::new()
    .get(&API_METHOD_GET_CONFIG)
    .put(&API_METHOD_UPDATE_DRIVE)
    .delete(&API_METHOD_DELETE_DRIVE)
    .subdirs(&[("scan", &SCAN_DRIVES)]);


pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_LIST_DRIVES)
    .post(&API_METHOD_CREATE_DRIVE)
    .match_all("name", &ITEM_ROUTER);
