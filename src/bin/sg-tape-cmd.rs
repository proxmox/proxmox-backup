/// Tape command implemented using scsi-generic raw commands
///
/// SCSI-generic command needs root privileges, so this binary need
/// to be setuid root.
///
/// This command can use STDIN as tape device handle.

use std::fs::File;
use std::os::unix::io::{AsRawFd, FromRawFd};

use anyhow::{bail, Error};
use serde_json::Value;

use proxmox::{
    api::{
        api,
        cli::*,
        RpcEnvironment,
    },
    tools::Uuid,
};

use proxmox_backup::{
    config,
    backup::Fingerprint,
    api2::types::{
        LINUX_DRIVE_PATH_SCHEMA,
        DRIVE_NAME_SCHEMA,
        TAPE_ENCRYPTION_KEY_FINGERPRINT_SCHEMA,
        MEDIA_SET_UUID_SCHEMA,
        LinuxTapeDrive,
    },
    tape::{
        drive::{
            TapeDriver,
            LinuxTapeHandle,
            open_linux_tape_device,
            check_tape_is_linux_tape_device,
        },
    },
};

fn get_tape_handle(param: &Value) -> Result<LinuxTapeHandle, Error> {

    let handle = if let Some(name) = param["drive"].as_str() {
        let (config, _digest) = config::drive::config()?;
        let drive: LinuxTapeDrive = config.lookup("linux", &name)?;
        eprintln!("using device {}", drive.path);
        drive.open()?
    } else if let Some(device) = param["device"].as_str() {
        eprintln!("using device {}", device);
        LinuxTapeHandle::new(open_linux_tape_device(&device)?)
    } else if let Some(true) = param["stdin"].as_bool() {
        eprintln!("using stdin");
        let fd = std::io::stdin().as_raw_fd();
        let file = unsafe { File::from_raw_fd(fd) };
        check_tape_is_linux_tape_device(&file)?;
        LinuxTapeHandle::new(file)
    } else if let Ok(name) = std::env::var("PROXMOX_TAPE_DRIVE") {
        let (config, _digest) = config::drive::config()?;
        let drive: LinuxTapeDrive = config.lookup("linux", &name)?;
        eprintln!("using device {}", drive.path);
        drive.open()?
    } else {
        let (config, _digest) = config::drive::config()?;

        let mut drive_names = Vec::new();
        for (name, (section_type, _)) in config.sections.iter() {
            if section_type != "linux" { continue; }
            drive_names.push(name);
        }

        if drive_names.len() == 1 {
            let name = drive_names[0];
            let drive: LinuxTapeDrive = config.lookup("linux", &name)?;
            eprintln!("using device {}", drive.path);
            drive.open()?
        } else {
            bail!("no drive/device specified");
        }
    };

    Ok(handle)
}

#[api(
   input: {
        properties: {
            drive: {
                schema: DRIVE_NAME_SCHEMA,
                optional: true,
            },
            device: {
                schema: LINUX_DRIVE_PATH_SCHEMA,
                optional: true,
            },
            stdin: {
                description: "Use standard input as device handle.",
                type: bool,
                optional: true,
            },
        },
    },
)]
/// Tape/Media Status
fn status(
    param: Value,
) -> Result<(), Error> {

    let result = proxmox::try_block!({
        let mut handle = get_tape_handle(&param)?;
        handle.get_drive_and_media_status()
   }).map_err(|err: Error| err.to_string());

    println!("{}", serde_json::to_string_pretty(&result)?);

    Ok(())
}

#[api(
   input: {
        properties: {
            drive: {
                schema: DRIVE_NAME_SCHEMA,
                optional: true,
            },
            device: {
                schema: LINUX_DRIVE_PATH_SCHEMA,
                optional: true,
            },
            stdin: {
                description: "Use standard input as device handle.",
                type: bool,
                optional: true,
            },
        },
    },
)]
/// Read Cartridge Memory (Medium auxiliary memory attributes)
fn cartridge_memory(
    param: Value,
) -> Result<(), Error> {

    let result = proxmox::try_block!({
        let mut handle = get_tape_handle(&param)?;

        handle.cartridge_memory()
    }).map_err(|err| err.to_string());

    println!("{}", serde_json::to_string_pretty(&result)?);

    Ok(())
}

#[api(
   input: {
        properties: {
            drive: {
                schema: DRIVE_NAME_SCHEMA,
                optional: true,
            },
            device: {
                schema: LINUX_DRIVE_PATH_SCHEMA,
                optional: true,
            },
            stdin: {
                description: "Use standard input as device handle.",
                type: bool,
                optional: true,
            },
        },
    },
)]
/// Read Tape Alert Flags
fn tape_alert_flags(
    param: Value,
) -> Result<(), Error> {

    let result = proxmox::try_block!({
        let mut handle = get_tape_handle(&param)?;

        let flags = handle.tape_alert_flags()?;
        Ok(flags.bits())
    }).map_err(|err: Error| err.to_string());

    println!("{}", serde_json::to_string_pretty(&result)?);

    Ok(())
}

#[api(
    input: {
        properties: {
            fingerprint: {
                schema: TAPE_ENCRYPTION_KEY_FINGERPRINT_SCHEMA,
                optional: true,
            },
            uuid: {
                schema: MEDIA_SET_UUID_SCHEMA,
                optional: true,
            },
            drive: {
                schema: DRIVE_NAME_SCHEMA,
                optional: true,
            },
            device: {
                schema: LINUX_DRIVE_PATH_SCHEMA,
                optional: true,
            },
            stdin: {
                description: "Use standard input as device handle.",
                type: bool,
                optional: true,
            },
        },
    },
)]
/// Set or clear encryption key
fn set_encryption(
    fingerprint: Option<Fingerprint>,
    uuid: Option<Uuid>,
    param: Value,
) -> Result<(), Error> {

    let result = proxmox::try_block!({
        let mut handle = get_tape_handle(&param)?;

        match (fingerprint, uuid) {
            (Some(fingerprint), Some(uuid)) => {
                handle.set_encryption(Some((fingerprint, uuid)))?;
            }
            (Some(_), None) => {
                bail!("missing media set uuid");
            }
            (None, _) => {
                handle.set_encryption(None)?;
            }
        }

        Ok(())
    }).map_err(|err: Error| err.to_string());

    println!("{}", serde_json::to_string_pretty(&result)?);

    Ok(())
}

#[api(
   input: {
        properties: {
            drive: {
                schema: DRIVE_NAME_SCHEMA,
                optional: true,
            },
            device: {
                schema: LINUX_DRIVE_PATH_SCHEMA,
                optional: true,
            },
            stdin: {
                description: "Use standard input as device handle.",
                type: bool,
                optional: true,
            },
        },
    },
)]
/// Read volume statistics
fn volume_statistics(
    param: Value,
) -> Result<(), Error> {

    let result = proxmox::try_block!({
        let mut handle = get_tape_handle(&param)?;
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

    if !running_uid.is_root() && (running_uid != backup_uid || running_gid != backup_gid) {
        bail!(
            "Not running as backup user or group (got uid {} gid {})",
            running_uid, running_gid,
        );
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
        .insert(
            "encryption",
            CliCommand::new(&API_METHOD_SET_ENCRYPTION)
        )
        ;

    let mut rpcenv = CliEnvironment::new();
    rpcenv.set_auth_id(Some(String::from("root@pam")));

    run_cli_command(cmd_def, rpcenv, None);

    Ok(())
}
