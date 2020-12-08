use anyhow::{bail, Error};
use serde_json::Value;

use proxmox::api::{api, Router, RpcEnvironment};

use crate::{
    config,
    api2::types::{
        CHANGER_ID_SCHEMA,
        LINUX_DRIVE_PATH_SCHEMA,
        DriveListEntry,
        ScsiTapeChanger,
        TapeDeviceInfo,
    },
    tape::{
        linux_tape_changer_list,
        check_drive_path,
        lookup_drive,
    },
};

#[api(
    input: {
        properties: {
            name: {
                schema: CHANGER_ID_SCHEMA,
            },
            path: {
                schema: LINUX_DRIVE_PATH_SCHEMA,
            },
        },
    },
)]
/// Create a new changer device
pub fn create_changer(
    name: String,
    path: String,
) -> Result<(), Error> {

    let _lock = config::drive::lock()?;

    let (mut config, _digest) = config::drive::config()?;

    let linux_changers = linux_tape_changer_list();

    check_drive_path(&linux_changers, &path)?;

    if config.sections.get(&name).is_some() {
        bail!("Entry '{}' already exists", name);
    }

    let item = ScsiTapeChanger {
        name: name.clone(),
        path,
    };

    config.set_data(&name, "changer", &item)?;

    config::drive::save_config(&config)?;

    Ok(())
}

#[api(
    input: {
        properties: {
            name: {
                schema: CHANGER_ID_SCHEMA,
            },
        },
    },
    returns: {
        type: ScsiTapeChanger,
    },

)]
/// Get tape changer configuration
pub fn get_config(
    name: String,
    _param: Value,
    mut rpcenv: &mut dyn RpcEnvironment,
) -> Result<ScsiTapeChanger, Error> {

    let (config, digest) = config::drive::config()?;

    let data: ScsiTapeChanger = config.lookup("changer", &name)?;

    rpcenv["digest"] = proxmox::tools::digest_to_hex(&digest).into();

    Ok(data)
}

#[api(
    input: {
        properties: {},
    },
    returns: {
        description: "The list of configured changers (with config digest).",
        type: Array,
        items: {
            type: DriveListEntry,
        },
    },
)]
/// List changers
pub fn list_changers(
    _param: Value,
    mut rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<DriveListEntry>, Error> {

    let (config, digest) = config::drive::config()?;

    let linux_changers = linux_tape_changer_list();

    let changer_list: Vec<ScsiTapeChanger> = config.convert_to_typed_array("changer")?;

    let mut list = Vec::new();

    for changer in changer_list {
        let mut entry = DriveListEntry {
            name: changer.name,
            path: changer.path.clone(),
            changer: None,
            vendor: None,
            model: None,
            serial: None,
        };
        if let Some(info) = lookup_drive(&linux_changers, &changer.path) {
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
                schema: CHANGER_ID_SCHEMA,
            },
            path: {
                schema: LINUX_DRIVE_PATH_SCHEMA,
                optional: true,
            },
        },
    },
)]
/// Update a tape changer configuration
pub fn update_changer(
    name: String,
    path: Option<String>,
    _param: Value,
) -> Result<(), Error> {

    let _lock = config::drive::lock()?;

    let (mut config, _digest) = config::drive::config()?;

    let mut data: ScsiTapeChanger = config.lookup("changer", &name)?;

    if let Some(path) = path {
        let changers = linux_tape_changer_list();
        check_drive_path(&changers, &path)?;
        data.path = path;
    }

    config.set_data(&name, "changer", &data)?;

    config::drive::save_config(&config)?;

    Ok(())
}

#[api(
    input: {
        properties: {
            name: {
                schema: CHANGER_ID_SCHEMA,
            },
        },
    },
)]
/// Delete a tape changer configuration
pub fn delete_changer(name: String, _param: Value) -> Result<(), Error> {

    let _lock = config::drive::lock()?;

    let (mut config, _digest) = config::drive::config()?;

    match config.sections.get(&name) {
        Some((section_type, _)) => {
            if section_type != "changer" {
                bail!("Entry '{}' exists, but is not a changer device", name);
            }
            config.sections.remove(&name);
        },
        None => bail!("Delete changer '{}' failed - no such entry", name),
    }

    config::drive::save_config(&config)?;

    Ok(())
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

pub const SCAN_CHANGERS: Router = Router::new()
    .get(&API_METHOD_SCAN_CHANGERS);


const ITEM_ROUTER: Router = Router::new()
    .get(&API_METHOD_GET_CONFIG)
    .put(&API_METHOD_UPDATE_CHANGER)
    .delete(&API_METHOD_DELETE_CHANGER);


pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_LIST_CHANGERS)
    .post(&API_METHOD_CREATE_CHANGER)
    .match_all("name", &ITEM_ROUTER);
