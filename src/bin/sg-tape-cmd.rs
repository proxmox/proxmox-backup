/// Tape command implemented using scsi-generic raw commands
///
/// SCSI-generic command needs root priviledges, so this binary need
/// to be setuid root.
///
/// This command can use STDIN as tape device handle.

use std::fs::File;
use std::os::unix::io::{AsRawFd, FromRawFd};

use anyhow::{bail, Error};

use proxmox::{
    api::{
        api,
        cli::*,
        RpcEnvironment,
    },
};

use proxmox_backup::{
    api2::types::{
        LINUX_DRIVE_PATH_SCHEMA,
    },
    tape::{
        TapeDriver,
        linux_tape::{
            LinuxTapeHandle,
            open_linux_tape_device,
            check_tape_is_linux_tape_device,
        },
    },
};

fn get_tape_handle(device: Option<String>) -> Result<LinuxTapeHandle, Error> {

    let file = if let Some(device) = device {
        open_linux_tape_device(&device)?
    } else {
        let fd = std::io::stdin().as_raw_fd();
        let file = unsafe { File::from_raw_fd(fd) };
        check_tape_is_linux_tape_device(&file)?;
        file
    };
    Ok(LinuxTapeHandle::new(file))
}

#[api(
   input: {
        properties: {
            device: {
                schema: LINUX_DRIVE_PATH_SCHEMA,
                optional: true,
            },
        },
    },
)]
/// Tape/Media Status
fn status(
    device: Option<String>,
) -> Result<(), Error> {

    let result = proxmox::try_block!({
        let mut handle = get_tape_handle(device)?;
        handle.get_drive_and_media_status()
   }).map_err(|err: Error| err.to_string());

    println!("{}", serde_json::to_string_pretty(&result)?);

    Ok(())
}

#[api(
   input: {
        properties: {
            device: {
                schema: LINUX_DRIVE_PATH_SCHEMA,
                optional: true,
            },
        },
    },
)]
/// Read Cartridge Memory (Medium auxiliary memory attributes)
fn cartridge_memory(
    device: Option<String>,
) -> Result<(), Error> {

    let result = proxmox::try_block!({
        let mut handle = get_tape_handle(device)?;

        handle.cartridge_memory()
    }).map_err(|err| err.to_string());

    println!("{}", serde_json::to_string_pretty(&result)?);

    Ok(())
}

#[api(
   input: {
        properties: {
            device: {
                schema: LINUX_DRIVE_PATH_SCHEMA,
                optional: true,
            },
        },
    },
)]
/// Read Tape Alert Flags
fn tape_alert_flags(
    device: Option<String>,
) -> Result<(), Error> {

    let result = proxmox::try_block!({
        let mut handle = get_tape_handle(device)?;

        let flags = handle.tape_alert_flags()?;
        Ok(flags.bits())
    }).map_err(|err: Error| err.to_string());

    println!("{}", serde_json::to_string_pretty(&result)?);

    Ok(())
}

#[api(
   input: {
        properties: {
            device: {
                schema: LINUX_DRIVE_PATH_SCHEMA,
                optional: true,
            },
        },
    },
)]
/// Read volume statistics
fn volume_statistics(
    device: Option<String>,
) -> Result<(), Error> {

    let result = proxmox::try_block!({
        let mut handle = get_tape_handle(device)?;
        handle.volume_statistics()
    }).map_err(|err: Error| err.to_string());

    println!("{}", serde_json::to_string_pretty(&result)?);

    Ok(())
}

fn main() -> Result<(), Error> {

    // check if we are user root or backup
    let backup_uid = proxmox_backup::backup::backup_user()?.uid;
    let backup_gid = proxmox_backup::backup::backup_group()?.gid;
    let running_uid = nix::unistd::Uid::current();
    let running_gid = nix::unistd::Gid::current();

    let effective_uid = nix::unistd::Uid::effective();
    if !effective_uid.is_root() {
        bail!("this program needs to be run with setuid root");
    }

    if !running_uid.is_root() {
        if running_uid != backup_uid || running_gid != backup_gid {
            bail!(
                "Not running as backup user or group (got uid {} gid {})",
                running_uid, running_gid,
            );
        }
    }

    let cmd_def = CliCommandMap::new()
        .insert(
            "status",
            CliCommand::new(&API_METHOD_STATUS)
        )
        .insert(
            "cartridge-memory",
            CliCommand::new(&API_METHOD_CARTRIDGE_MEMORY)
        )
        .insert(
            "tape-alert-flags",
            CliCommand::new(&API_METHOD_TAPE_ALERT_FLAGS)
        )
        .insert(
            "volume-statistics",
            CliCommand::new(&API_METHOD_VOLUME_STATISTICS)
        )
        ;

    let mut rpcenv = CliEnvironment::new();
    rpcenv.set_auth_id(Some(String::from("root@pam")));

    run_cli_command(cmd_def, rpcenv, None);

    Ok(())
}
