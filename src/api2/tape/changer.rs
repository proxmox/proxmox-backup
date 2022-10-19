use std::collections::HashMap;

use anyhow::Error;
use serde_json::Value;

use proxmox_router::{list_subdirs_api_method, Permission, Router, RpcEnvironment, SubdirMap};
use proxmox_schema::api;

use pbs_api_types::{
    Authid, ChangerListEntry, LtoTapeDrive, MtxEntryKind, MtxStatusEntry, ScsiTapeChanger,
    CHANGER_NAME_SCHEMA, PRIV_TAPE_AUDIT, PRIV_TAPE_READ,
};
use pbs_config::CachedUserInfo;
use pbs_tape::{
    linux_list_drives::{linux_tape_changer_list, lookup_device_identification},
    ElementStatus,
};

use crate::tape::{
    changer::{mtx_status_to_online_set, OnlineStatusMap, ScsiMediaChange},
    drive::get_tape_device_state,
    Inventory, TAPE_STATUS_DIR,
};

#[api(
    input: {
        properties: {
            name: {
                schema: CHANGER_NAME_SCHEMA,
            },
            cache: {
                description: "Use cached value.",
                optional: true,
                default: true,
            },
        },
    },
    returns: {
        description: "A status entry for each drive and slot.",
        type: Array,
        items: {
            type: MtxStatusEntry,
        },
    },
    access: {
        permission: &Permission::Privilege(&["tape", "device", "{name}"], PRIV_TAPE_AUDIT, false),
    },
)]
/// Get tape changer status
pub async fn get_status(name: String, cache: bool) -> Result<Vec<MtxStatusEntry>, Error> {
    let (config, _digest) = pbs_config::drive::config()?;

    let mut changer_config: ScsiTapeChanger = config.lookup("changer", &name)?;

    let status = tokio::task::spawn_blocking(move || changer_config.status(cache)).await??;

    let mut inventory = Inventory::load(TAPE_STATUS_DIR)?;

    let mut map = OnlineStatusMap::new(&config)?;
    let online_set = mtx_status_to_online_set(&status, &inventory);
    map.update_online_status(&name, online_set)?;

    inventory.update_online_status(&map)?;

    let drive_list: Vec<LtoTapeDrive> = config.convert_to_typed_array("lto")?;
    let mut drive_map: HashMap<u64, String> = HashMap::new();

    for drive in drive_list {
        if let Some(changer) = drive.changer {
            if changer != name {
                continue;
            }
            let num = drive.changer_drivenum.unwrap_or(0);
            drive_map.insert(num, drive.name.clone());
        }
    }

    let mut list = Vec::new();

    for (id, drive_status) in status.drives.iter().enumerate() {
        let mut state = None;
        if let Some(drive) = drive_map.get(&(id as u64)) {
            state = get_tape_device_state(&config, drive)?;
        }
        let entry = MtxStatusEntry {
            entry_kind: MtxEntryKind::Drive,
            entry_id: id as u64,
            label_text: match &drive_status.status {
                ElementStatus::Empty => None,
                ElementStatus::Full => Some(String::new()),
                ElementStatus::VolumeTag(tag) => Some(tag.to_string()),
            },
            loaded_slot: drive_status.loaded_slot,
            state,
        };
        list.push(entry);
    }

    for (id, slot_info) in status.slots.iter().enumerate() {
        let entry = MtxStatusEntry {
            entry_kind: if slot_info.import_export {
                MtxEntryKind::ImportExport
            } else {
                MtxEntryKind::Slot
            },
            entry_id: id as u64 + 1,
            label_text: match &slot_info.status {
                ElementStatus::Empty => None,
                ElementStatus::Full => Some(String::new()),
                ElementStatus::VolumeTag(tag) => Some(tag.to_string()),
            },
            loaded_slot: None,
            state: None,
        };
        list.push(entry);
    }

    Ok(list)
}

#[api(
    input: {
        properties: {
            name: {
                schema: CHANGER_NAME_SCHEMA,
            },
            from: {
                description: "Source slot number",
                minimum: 1,
            },
            to: {
                description: "Destination slot number",
                minimum: 1,
            },
        },
    },
    access: {
        permission: &Permission::Privilege(&["tape", "device", "{name}"], PRIV_TAPE_READ, false),
    },
)]
/// Transfers media from one slot to another
pub async fn transfer(name: String, from: u64, to: u64) -> Result<(), Error> {
    let (config, _digest) = pbs_config::drive::config()?;

    let mut changer_config: ScsiTapeChanger = config.lookup("changer", &name)?;

    tokio::task::spawn_blocking(move || {
        changer_config.transfer(from, to)?;
        Ok(())
    })
    .await?
}

#[api(
    input: {
        properties: {},
    },
    returns: {
        description: "The list of configured changers with model information.",
        type: Array,
        items: {
            type: ChangerListEntry,
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
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<ChangerListEntry>, Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let user_info = CachedUserInfo::new()?;

    let (config, _digest) = pbs_config::drive::config()?;

    let linux_changers = linux_tape_changer_list();

    let changer_list: Vec<ScsiTapeChanger> = config.convert_to_typed_array("changer")?;

    let mut list = Vec::new();

    for changer in changer_list {
        let privs = user_info.lookup_privs(&auth_id, &["tape", "changer", &changer.name]);
        if (privs & PRIV_TAPE_AUDIT) == 0 {
            continue;
        }

        let info = lookup_device_identification(&linux_changers, &changer.path);
        let entry = ChangerListEntry {
            config: changer,
            info,
        };
        list.push(entry);
    }
    Ok(list)
}

const SUBDIRS: SubdirMap = &[
    ("status", &Router::new().get(&API_METHOD_GET_STATUS)),
    ("transfer", &Router::new().post(&API_METHOD_TRANSFER)),
];

const ITEM_ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(SUBDIRS))
    .subdirs(SUBDIRS);

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_LIST_CHANGERS)
    .match_all("name", &ITEM_ROUTER);
