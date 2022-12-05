use anyhow::Error;

use pbs_api_types::ScsiTapeChanger;
use pbs_tape::MtxStatus;
use proxmox_sys::command::run_command;

use crate::tape::changer::mtx::parse_mtx_status;

/// Run 'mtx status' and return parsed result.
pub fn mtx_status(config: &ScsiTapeChanger) -> Result<MtxStatus, Error> {
    let path = &config.path;

    let mut command = std::process::Command::new("mtx");
    command.args(["-f", path, "status"]);

    let output = run_command(command, None)?;

    let mut status = parse_mtx_status(&output)?;

    status.mark_import_export_slots(config)?;

    Ok(status)
}

/// Run 'mtx load'
pub fn mtx_load(path: &str, slot: u64, drivenum: u64) -> Result<(), Error> {
    let mut command = std::process::Command::new("mtx");
    command.args(["-f", path, "load", &slot.to_string(), &drivenum.to_string()]);
    run_command(command, None)?;

    Ok(())
}

/// Run 'mtx unload'
pub fn mtx_unload(path: &str, slot: u64, drivenum: u64) -> Result<(), Error> {
    let mut command = std::process::Command::new("mtx");
    command.args([
        "-f",
        path,
        "unload",
        &slot.to_string(),
        &drivenum.to_string(),
    ]);
    run_command(command, None)?;

    Ok(())
}

/// Run 'mtx transfer'
pub fn mtx_transfer(path: &str, from_slot: u64, to_slot: u64) -> Result<(), Error> {
    let mut command = std::process::Command::new("mtx");
    command.args([
        "-f",
        path,
        "transfer",
        &from_slot.to_string(),
        &to_slot.to_string(),
    ]);

    run_command(command, None)?;

    Ok(())
}
