use std::collections::HashSet;

use anyhow::Error;
use serde_json::Value;

use proxmox::{
    tools::Uuid,
    api::schema::parse_property_string,
};

use crate::{
    tools::run_command,
    api2::types::{
        SLOT_ARRAY_SCHEMA,
        ScsiTapeChanger,
    },
    tape::{
        Inventory,
        changer::{
            MtxStatus,
            ElementStatus,
            parse_mtx_status,
        },
    },
};

/// Run 'mtx status' and return parsed result.
pub fn mtx_status(config: &ScsiTapeChanger) -> Result<MtxStatus, Error> {

    let path = &config.path;

    let mut export_slots: HashSet<u64> = HashSet::new();

    if let Some(slots) = &config.export_slots {
        let slots: Value = parse_property_string(&slots, &SLOT_ARRAY_SCHEMA)?;
        export_slots = slots
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_u64())
            .collect();
    }

    let mut command = std::process::Command::new("mtx");
    command.args(&["-f", path, "status"]);

    let output = run_command(command, None)?;

    let mut status = parse_mtx_status(&output)?;

    for (i, entry) in status.slots.iter_mut().enumerate() {
        let slot = i as u64 + 1;
        if export_slots.contains(&slot) {
            entry.0 = true; // mark as IMPORT/EXPORT
        }
    }

    Ok(status)
}

/// Run 'mtx load'
pub fn mtx_load(
    path: &str,
    slot: u64,
    drivenum: u64,
) -> Result<(), Error> {

    let mut command = std::process::Command::new("mtx");
    command.args(&["-f", path, "load", &slot.to_string(), &drivenum.to_string()]);
    run_command(command, None)?;

    Ok(())
}

/// Run 'mtx unload'
pub fn mtx_unload(
    path: &str,
    slot: u64,
    drivenum: u64,
) -> Result<(), Error> {

    let mut command = std::process::Command::new("mtx");
    command.args(&["-f", path, "unload", &slot.to_string(), &drivenum.to_string()]);
    run_command(command, None)?;

    Ok(())
}

/// Run 'mtx transfer'
pub fn mtx_transfer(
    path: &str,
    from_slot: u64,
    to_slot: u64,
) -> Result<(), Error> {

    let mut command = std::process::Command::new("mtx");
    command.args(&["-f", path, "transfer", &from_slot.to_string(), &to_slot.to_string()]);

    run_command(command, None)?;

    Ok(())
}

/// Extract the list of online media from MtxStatus
///
/// Returns a HashSet containing all found media Uuid
pub fn mtx_status_to_online_set(status: &MtxStatus, inventory: &Inventory) -> HashSet<Uuid> {

    let mut online_set = HashSet::new();

    for drive_status in status.drives.iter() {
        if let ElementStatus::VolumeTag(ref changer_id) = drive_status.status {
            if let Some(media_id) = inventory.find_media_by_changer_id(changer_id) {
                online_set.insert(media_id.label.uuid.clone());
            }
        }
    }

    for (import_export, slot_status) in status.slots.iter() {
        if *import_export { continue; }
        if let ElementStatus::VolumeTag(ref changer_id) = slot_status {
            if let Some(media_id) = inventory.find_media_by_changer_id(changer_id) {
                online_set.insert(media_id.label.uuid.clone());
            }
        }
    }

    online_set
}
