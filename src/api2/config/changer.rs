use anyhow::{bail, Error};
use ::serde::{Deserialize, Serialize};
use serde_json::Value;

use proxmox::api::{
    api,
    Router,
    RpcEnvironment,
    Permission,
    schema::parse_property_string,
};

use pbs_api_types::{
    Authid, ScsiTapeChanger, ScsiTapeChangerUpdater, LtoTapeDrive,
    PROXMOX_CONFIG_DIGEST_SCHEMA, CHANGER_NAME_SCHEMA, SLOT_ARRAY_SCHEMA,
    PRIV_TAPE_AUDIT, PRIV_TAPE_MODIFY,
};

use crate::{
    config::cached_user_info::CachedUserInfo,
    tape::{
        linux_tape_changer_list,
        check_drive_path,
    },
};

#[api(
    protected: true,
    input: {
        properties: {
            config: {
                type: ScsiTapeChanger,
                flatten: true,
            },
        },
    },
    access: {
        permission: &Permission::Privilege(&["tape", "device"], PRIV_TAPE_MODIFY, false),
    },
)]
/// Create a new changer device
pub fn create_changer(config: ScsiTapeChanger) -> Result<(), Error> {

    let _lock = pbs_config::drive::lock()?;

    let (mut section_config, _digest) = pbs_config::drive::config()?;

    let linux_changers = linux_tape_changer_list();

    check_drive_path(&linux_changers, &config.path)?;

    let existing: Vec<ScsiTapeChanger> = section_config.convert_to_typed_array("changer")?;

    for changer in existing {
        if changer.name == config.name {
            bail!("Entry '{}' already exists", config.name);
        }

        if changer.path == config.path {
            bail!("Path '{}' already in use by '{}'", config.path, changer.name);
        }
    }

    section_config.set_data(&config.name, "changer", &config)?;

    pbs_config::drive::save_config(&section_config)?;

    Ok(())
}

#[api(
    input: {
        properties: {
            name: {
                schema: CHANGER_NAME_SCHEMA,
            },
        },
    },
    returns: {
        type: ScsiTapeChanger,
    },
    access: {
        permission: &Permission::Privilege(&["tape", "device", "{name}"], PRIV_TAPE_AUDIT, false),
    },
)]
/// Get tape changer configuration
pub fn get_config(
    name: String,
    _param: Value,
    mut rpcenv: &mut dyn RpcEnvironment,
) -> Result<ScsiTapeChanger, Error> {

    let (config, digest) = pbs_config::drive::config()?;

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
            type: ScsiTapeChanger,
        },
    },
    access: {
        description: "List configured tape changer filtered by Tape.Audit privileges",
        permission: &Permission::Anybody,
    },
)]
/// List changers
pub fn list_changers(
    _param: Value,
    mut rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<ScsiTapeChanger>, Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let user_info = CachedUserInfo::new()?;

    let (config, digest) = pbs_config::drive::config()?;

    let list: Vec<ScsiTapeChanger> = config.convert_to_typed_array("changer")?;

    let list = list
        .into_iter()
        .filter(|changer| {
            let privs = user_info.lookup_privs(&auth_id, &["tape", "device", &changer.name]);
            privs & PRIV_TAPE_AUDIT != 0
        })
        .collect();

    rpcenv["digest"] = proxmox::tools::digest_to_hex(&digest).into();

    Ok(list)
}
#[api()]
#[derive(Serialize, Deserialize)]
#[allow(non_camel_case_types)]
#[serde(rename_all = "kebab-case")]
/// Deletable property name
pub enum DeletableProperty {
    /// Delete export-slots.
    export_slots,
}

#[api(
    protected: true,
    input: {
        properties: {
            name: {
                schema: CHANGER_NAME_SCHEMA,
            },
            update: {
                type: ScsiTapeChangerUpdater,
                flatten: true,
            },
            delete: {
                description: "List of properties to delete.",
                type: Array,
                optional: true,
                items: {
                    type: DeletableProperty,
                },
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
/// Update a tape changer configuration
pub fn update_changer(
    name: String,
    update: ScsiTapeChangerUpdater,
    delete: Option<Vec<DeletableProperty>>,
    digest: Option<String>,
    _param: Value,
) -> Result<(), Error> {

    let _lock = pbs_config::drive::lock()?;

    let (mut config, expected_digest) = pbs_config::drive::config()?;

    if let Some(ref digest) = digest {
        let digest = proxmox::tools::hex_to_digest(digest)?;
        crate::tools::detect_modified_configuration_file(&digest, &expected_digest)?;
    }

    let mut data: ScsiTapeChanger = config.lookup("changer", &name)?;

    if let Some(delete) = delete {
        for delete_prop in delete {
            match delete_prop {
                DeletableProperty::export_slots => {
                    data.export_slots = None;
                }
            }
        }
    }

    if let Some(path) = update.path {
        let changers = linux_tape_changer_list();
        check_drive_path(&changers, &path)?;
        data.path = path;
    }

    if let Some(export_slots) = update.export_slots {
        let slots: Value = parse_property_string(
            &export_slots, &SLOT_ARRAY_SCHEMA
        )?;
        let mut slots: Vec<String> = slots
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.to_string())
            .collect();
        slots.sort();

        if slots.is_empty() {
            data.export_slots = None;
        } else {
            let slots = slots.join(",");
            data.export_slots = Some(slots);
        }
    }

    config.set_data(&name, "changer", &data)?;

    pbs_config::drive::save_config(&config)?;

    Ok(())
}

#[api(
    protected: true,
    input: {
        properties: {
            name: {
                schema: CHANGER_NAME_SCHEMA,
            },
        },
    },
    access: {
        permission: &Permission::Privilege(&["tape", "device", "{name}"], PRIV_TAPE_MODIFY, false),
    },
)]
/// Delete a tape changer configuration
pub fn delete_changer(name: String, _param: Value) -> Result<(), Error> {

    let _lock = pbs_config::drive::lock()?;

    let (mut config, _digest) = pbs_config::drive::config()?;

    match config.sections.get(&name) {
        Some((section_type, _)) => {
            if section_type != "changer" {
                bail!("Entry '{}' exists, but is not a changer device", name);
            }
            config.sections.remove(&name);
        },
        None => bail!("Delete changer '{}' failed - no such entry", name),
    }

    let drive_list: Vec<LtoTapeDrive> = config.convert_to_typed_array("lto")?;
    for drive in drive_list {
        if let Some(changer) = drive.changer {
            if changer == name {
                bail!("Delete changer '{}' failed - used by drive '{}'", name, drive.name);
            }
        }
    }

    pbs_config::drive::save_config(&config)?;

    Ok(())
}

const ITEM_ROUTER: Router = Router::new()
    .get(&API_METHOD_GET_CONFIG)
    .put(&API_METHOD_UPDATE_CHANGER)
    .delete(&API_METHOD_DELETE_CHANGER);


pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_LIST_CHANGERS)
    .post(&API_METHOD_CREATE_CHANGER)
    .match_all("name", &ITEM_ROUTER);
