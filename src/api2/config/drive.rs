use anyhow::{bail, Error};
use ::serde::{Deserialize, Serialize};
use serde_json::Value;

use proxmox::api::{api, Router, RpcEnvironment};

use crate::{
    config,
    api2::types::{
        PROXMOX_CONFIG_DIGEST_SCHEMA,
        DRIVE_ID_SCHEMA,
        CHANGER_ID_SCHEMA,
        CHANGER_DRIVE_ID_SCHEMA,
        LINUX_DRIVE_PATH_SCHEMA,
        DriveListEntry,
        LinuxTapeDrive,
        ScsiTapeChanger,
    },
    tape::{
        linux_tape_device_list,
        check_drive_path,
        lookup_drive,
    },
};

#[api(
    protected: true,
    input: {
        properties: {
            name: {
                schema: DRIVE_ID_SCHEMA,
            },
            path: {
                schema: LINUX_DRIVE_PATH_SCHEMA,
            },
            changer: {
                schema: CHANGER_ID_SCHEMA,
                optional: true,
            },
            "changer-drive-id": {
                schema: CHANGER_DRIVE_ID_SCHEMA,
                optional: true,
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

#[api()]
#[derive(Serialize, Deserialize)]
#[allow(non_camel_case_types)]
#[serde(rename_all = "kebab-case")]
/// Deletable property name
pub enum DeletableProperty {
    /// Delete the changer property.
    changer,
    /// Delete the changer-drive-id property.
    changer_drive_id,
}

#[api(
    protected: true,
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
            "changer-drive-id": {
                schema: CHANGER_DRIVE_ID_SCHEMA,
                optional: true,
            },
            delete: {
                description: "List of properties to delete.",
                type: Array,
                optional: true,
                items: {
                    type: DeletableProperty,
                }
            },
            digest: {
                schema: PROXMOX_CONFIG_DIGEST_SCHEMA,
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
    changer_drive_id: Option<u64>,
    delete: Option<Vec<DeletableProperty>>,
    digest: Option<String>,
   _param: Value,
) -> Result<(), Error> {

    let _lock = config::drive::lock()?;

    let (mut config, expected_digest) = config::drive::config()?;

    if let Some(ref digest) = digest {
        let digest = proxmox::tools::hex_to_digest(digest)?;
        crate::tools::detect_modified_configuration_file(&digest, &expected_digest)?;
    }

    let mut data: LinuxTapeDrive = config.lookup("linux", &name)?;

    if let Some(delete) = delete {
        for delete_prop in delete {
            match delete_prop {
                DeletableProperty::changer => {
                    data.changer = None;
                    data.changer_drive_id = None;
                },
                DeletableProperty::changer_drive_id => { data.changer_drive_id = None; },
            }
        }
    }

    if let Some(path) = path {
        let linux_drives = linux_tape_device_list();
        check_drive_path(&linux_drives, &path)?;
        data.path = path;
    }

    if let Some(changer) = changer {
        let _: ScsiTapeChanger = config.lookup("changer", &changer)?;
        data.changer = Some(changer);
    }

    if let Some(changer_drive_id) = changer_drive_id {
        if changer_drive_id == 0 {
            data.changer_drive_id = None;
        } else {
            if data.changer.is_none() {
                bail!("Option 'changer-drive-id' requires option 'changer'.");
            }
            data.changer_drive_id = Some(changer_drive_id);
        }
    }

    config.set_data(&name, "linux", &data)?;

    config::drive::save_config(&config)?;

    Ok(())
}

#[api(
    protected: true,
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

const ITEM_ROUTER: Router = Router::new()
    .get(&API_METHOD_GET_CONFIG)
    .put(&API_METHOD_UPDATE_DRIVE)
    .delete(&API_METHOD_DELETE_DRIVE);


pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_LIST_DRIVES)
    .post(&API_METHOD_CREATE_DRIVE)
    .match_all("name", &ITEM_ROUTER);
