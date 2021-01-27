use std::path::Path;

use anyhow::Error;
use serde_json::Value;

use proxmox::api::{api, Router, SubdirMap};
use proxmox::list_subdirs_api_method;

use crate::{
    config,
    api2::types::{
        CHANGER_NAME_SCHEMA,
        DriveListEntry,
        ScsiTapeChanger,
        MtxStatusEntry,
        MtxEntryKind,
    },
    tape::{
        TAPE_STATUS_DIR,
        Inventory,
        linux_tape_changer_list,
        changer::{
            OnlineStatusMap,
            ElementStatus,
            ScsiMediaChange,
            mtx_status_to_online_set,
        },
        lookup_drive,
    },
};


#[api(
    input: {
        properties: {
            name: {
                schema: CHANGER_NAME_SCHEMA,
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
)]
/// Get tape changer status
pub async fn get_status(name: String) -> Result<Vec<MtxStatusEntry>, Error> {

    let (config, _digest) = config::drive::config()?;

    let mut changer_config: ScsiTapeChanger = config.lookup("changer", &name)?;

    let status = tokio::task::spawn_blocking(move || {
        changer_config.status()
    }).await??;

    let state_path = Path::new(TAPE_STATUS_DIR);
    let mut inventory = Inventory::load(state_path)?;

    let mut map = OnlineStatusMap::new(&config)?;
    let online_set = mtx_status_to_online_set(&status, &inventory);
    map.update_online_status(&name, online_set)?;

    inventory.update_online_status(&map)?;

    let mut list = Vec::new();

    for (id, drive_status) in status.drives.iter().enumerate() {
        let entry = MtxStatusEntry {
            entry_kind: MtxEntryKind::Drive,
            entry_id: id as u64,
            label_text: match &drive_status.status {
                ElementStatus::Empty => None,
                ElementStatus::Full => Some(String::new()),
                ElementStatus::VolumeTag(tag) => Some(tag.to_string()),
            },
            loaded_slot: drive_status.loaded_slot,
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
)]
/// Transfers media from one slot to another
pub async fn transfer(
    name: String,
    from: u64,
    to: u64,
) -> Result<(), Error> {

    let (config, _digest) = config::drive::config()?;

    let mut changer_config: ScsiTapeChanger = config.lookup("changer", &name)?;

    tokio::task::spawn_blocking(move || {
        changer_config.transfer(from, to)
    }).await?
}

#[api(
    input: {
        properties: {},
    },
    returns: {
        description: "The list of configured changers with model information.",
        type: Array,
        items: {
            type: DriveListEntry,
        },
    },
)]
/// List changers
pub fn list_changers(
    _param: Value,
) -> Result<Vec<DriveListEntry>, Error> {

    let (config, _digest) = config::drive::config()?;

    let linux_changers = linux_tape_changer_list();

    let changer_list: Vec<ScsiTapeChanger> = config.convert_to_typed_array("changer")?;

    let mut list = Vec::new();

    for changer in changer_list {
        let mut entry = DriveListEntry {
            name: changer.name,
            path: changer.path.clone(),
            changer: None,
            changer_drivenum: None,
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
    Ok(list)
}

const SUBDIRS: SubdirMap = &[
    (
        "status",
        &Router::new()
            .get(&API_METHOD_GET_STATUS)
    ),
    (
        "transfer",
        &Router::new()
            .post(&API_METHOD_TRANSFER)
    ),
];

const ITEM_ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(SUBDIRS))
    .subdirs(&SUBDIRS);

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_LIST_CHANGERS)
    .match_all("name", &ITEM_ROUTER);
