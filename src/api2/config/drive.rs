use ::serde::{Deserialize, Serialize};
use anyhow::{format_err, Error};
use hex::FromHex;
use serde_json::Value;

use proxmox_router::{http_bail, Permission, Router, RpcEnvironment};
use proxmox_schema::{api, param_bail};

use pbs_api_types::{
    Authid, LtoTapeDrive, LtoTapeDriveUpdater, ScsiTapeChanger, DRIVE_NAME_SCHEMA, PRIV_TAPE_AUDIT,
    PRIV_TAPE_MODIFY, PROXMOX_CONFIG_DIGEST_SCHEMA,
};
use pbs_config::CachedUserInfo;

use pbs_tape::linux_list_drives::{check_drive_path, lto_tape_device_list};

#[api(
    protected: true,
    input: {
        properties: {
            config: {
                type: LtoTapeDrive,
                flatten: true,
            },
        },
    },
    access: {
        permission: &Permission::Privilege(&["tape", "device"], PRIV_TAPE_MODIFY, false),
    },
)]
/// Create a new drive
pub fn create_drive(config: LtoTapeDrive) -> Result<(), Error> {
    let _lock = pbs_config::drive::lock()?;

    let (mut section_config, _digest) = pbs_config::drive::config()?;

    let lto_drives = lto_tape_device_list();

    check_drive_path(&lto_drives, &config.path)?;

    let existing: Vec<LtoTapeDrive> = section_config.convert_to_typed_array("lto")?;

    for drive in existing {
        if drive.name == config.name {
            param_bail!("name", "Entry '{}' already exists", config.name);
        }
        if drive.path == config.path {
            param_bail!(
                "path",
                "Path '{}' already used in drive '{}'",
                config.path,
                drive.name
            );
        }
    }

    section_config.set_data(&config.name, "lto", &config)?;

    pbs_config::drive::save_config(&section_config)?;

    Ok(())
}

#[api(
    input: {
        properties: {
            name: {
                schema: DRIVE_NAME_SCHEMA,
            },
        },
    },
    returns: {
        type: LtoTapeDrive,
    },
    access: {
        permission: &Permission::Privilege(&["tape", "device", "{name}"], PRIV_TAPE_AUDIT, false),
    },
)]
/// Get drive configuration
pub fn get_config(
    name: String,
    _param: Value,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<LtoTapeDrive, Error> {
    let (config, digest) = pbs_config::drive::config()?;

    let data: LtoTapeDrive = config.lookup("lto", &name)?;

    rpcenv["digest"] = hex::encode(digest).into();

    Ok(data)
}

#[api(
    input: {
        properties: {},
    },
    returns: {
        description: "The list of configured drives (with config digest).",
        type: Array,
        items: {
            type: LtoTapeDrive,
        },
    },
    access: {
        description: "List configured tape drives filtered by Tape.Audit privileges",
        permission: &Permission::Anybody,
    },
)]
/// List drives
pub fn list_drives(
    _param: Value,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<LtoTapeDrive>, Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let user_info = CachedUserInfo::new()?;

    let (config, digest) = pbs_config::drive::config()?;

    let drive_list: Vec<LtoTapeDrive> = config.convert_to_typed_array("lto")?;

    let drive_list = drive_list
        .into_iter()
        .filter(|drive| {
            let privs = user_info.lookup_privs(&auth_id, &["tape", "device", &drive.name]);
            privs & PRIV_TAPE_AUDIT != 0
        })
        .collect();

    rpcenv["digest"] = hex::encode(digest).into();

    Ok(drive_list)
}

#[api()]
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
/// Deletable property name
pub enum DeletableProperty {
    /// Delete the changer property.
    Changer,
    /// Delete the changer-drivenum property.
    ChangerDrivenum,
}

#[api(
    protected: true,
    input: {
        properties: {
            name: {
                schema: DRIVE_NAME_SCHEMA,
            },
            update: {
                type: LtoTapeDriveUpdater,
                flatten: true,
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
    access: {
        permission: &Permission::Privilege(&["tape", "device", "{name}"], PRIV_TAPE_MODIFY, false),
    },
)]
/// Update a drive configuration
pub fn update_drive(
    name: String,
    update: LtoTapeDriveUpdater,
    delete: Option<Vec<DeletableProperty>>,
    digest: Option<String>,
    _param: Value,
) -> Result<(), Error> {
    let _lock = pbs_config::drive::lock()?;

    let (mut config, expected_digest) = pbs_config::drive::config()?;

    if let Some(ref digest) = digest {
        let digest = <[u8; 32]>::from_hex(digest)?;
        crate::tools::detect_modified_configuration_file(&digest, &expected_digest)?;
    }

    let mut data: LtoTapeDrive = config.lookup("lto", &name)?;

    if let Some(delete) = delete {
        for delete_prop in delete {
            match delete_prop {
                DeletableProperty::Changer => {
                    data.changer = None;
                    data.changer_drivenum = None;
                }
                DeletableProperty::ChangerDrivenum => {
                    data.changer_drivenum = None;
                }
            }
        }
    }

    if let Some(path) = update.path {
        let lto_drives = lto_tape_device_list();
        check_drive_path(&lto_drives, &path)?;
        data.path = path;
    }

    if let Some(changer) = update.changer {
        let _: ScsiTapeChanger = config.lookup("changer", &changer)?;
        data.changer = Some(changer);
    }

    if let Some(changer_drivenum) = update.changer_drivenum {
        if changer_drivenum == 0 {
            data.changer_drivenum = None;
        } else {
            if data.changer.is_none() {
                param_bail!(
                    "changer",
                    format_err!("Option 'changer-drivenum' requires option 'changer'.")
                );
            }
            data.changer_drivenum = Some(changer_drivenum);
        }
    }

    config.set_data(&name, "lto", &data)?;

    pbs_config::drive::save_config(&config)?;

    Ok(())
}

#[api(
    protected: true,
    input: {
        properties: {
            name: {
                schema: DRIVE_NAME_SCHEMA,
            },
        },
    },
    access: {
        permission: &Permission::Privilege(&["tape", "device", "{name}"], PRIV_TAPE_MODIFY, false),
    },
)]
/// Delete a drive configuration
pub fn delete_drive(name: String, _param: Value) -> Result<(), Error> {
    let _lock = pbs_config::drive::lock()?;

    let (mut config, _digest) = pbs_config::drive::config()?;

    match config.sections.get(&name) {
        Some((section_type, _)) => {
            if section_type != "lto" {
                param_bail!(
                    "name",
                    "Entry '{}' exists, but is not a lto tape drive",
                    name
                );
            }
            config.sections.remove(&name);
        }
        None => http_bail!(NOT_FOUND, "Delete drive '{}' failed - no such drive", name),
    }

    pbs_config::drive::save_config(&config)?;

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
