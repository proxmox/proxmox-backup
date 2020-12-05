use std::collections::HashSet;

use anyhow::Error;

use proxmox::tools::Uuid;

use crate::{
    tools::run_command,
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
pub fn mtx_status(path: &str) -> Result<MtxStatus, Error> {

    let mut command = std::process::Command::new("mtx");
    command.args(&["-f", path, "status"]);

    let output = run_command(command, None)?;

    let status = parse_mtx_status(&output)?;

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

    for slot_status in status.slots.iter() {
        if let ElementStatus::VolumeTag(ref changer_id) = slot_status {
            if let Some(media_id) = inventory.find_media_by_changer_id(changer_id) {
                online_set.insert(media_id.label.uuid.clone());
            }
        }
    }

    online_set
}
