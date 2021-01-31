/// Control magnetic tape drive operation
///
/// This is a Rust implementation, meant to replace the 'mt' command
/// line tool.
///
/// Features:
///
/// - written in Rust
/// - optional json output format
/// - support tape alert flags
/// - support volume statistics
/// - read cartridge memory

use std::fs::File;

use anyhow::{bail, Error};
use serde_json::Value;

use proxmox::{
    api::{
        api,
        cli::*,
        RpcEnvironment,
    },
};

use proxmox_backup::{
    tools::sgutils2::{
        scsi_inquiry,
    },
    config::{
        self,
        drive::complete_drive_name,
    },
    backup::Fingerprint,
    api2::types::{
        LINUX_DRIVE_PATH_SCHEMA,
        DRIVE_NAME_SCHEMA,
        TAPE_ENCRYPTION_KEY_FINGERPRINT_SCHEMA,
        MEDIA_SET_UUID_SCHEMA,
        LinuxTapeDrive,
    },
    tape::{
        complete_drive_path,
        linux_tape_device_list,
        drive::{
            TapeDriver,
            LinuxTapeHandle,
            open_linux_tape_device,
            check_tape_is_linux_tape_device,
        },
    },
};

fn get_tape_handle(param: &Value) -> Result<LinuxTapeHandle, Error> {

    if let Some(name) = param["drive"].as_str() {
        let (config, _digest) = config::drive::config()?;
        let drive: LinuxTapeDrive = config.lookup("linux", &name)?;
        eprintln!("using device {}", drive.path);
        return drive.open();
    }

    if let Some(device) = param["device"].as_str() {
        eprintln!("using device {}", device);
        return Ok(LinuxTapeHandle::new(open_linux_tape_device(&device)?))
    }

    if let Ok(name) = std::env::var("PROXMOX_TAPE_DRIVE") {
        let (config, _digest) = config::drive::config()?;
        let drive: LinuxTapeDrive = config.lookup("linux", &name)?;
        eprintln!("using device {}", drive.path);
        return drive.open();
    }

    if let Ok(device) = std::env::var("TAPE") {
        eprintln!("using device {}", device);
        return Ok(LinuxTapeHandle::new(open_linux_tape_device(&device)?))
    }

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
        return drive.open();
    }

    bail!("no drive/device specified");
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
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        },
    },
)]
/// Read Cartridge Memory
fn cartridge_memory(param: Value) -> Result<(), Error> {

    let output_format = get_output_format(&param);

    let mut handle = get_tape_handle(&param)?;
    let result = handle.cartridge_memory();

    if output_format == "json-pretty" {
        let result = result.map_err(|err: Error| err.to_string());
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }

    if output_format == "json" {
        let result = result.map_err(|err: Error| err.to_string());
        println!("{}", serde_json::to_string(&result)?);
        return Ok(());
    }

    if output_format != "text" {
        bail!("unknown output format '{}'", output_format);
    }

    let list = result?;

    for item in list {
        println!("{}|{}|{}", item.id, item.name, item.value);
    }

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
       },
    },
)]
/// Eject drive media
fn eject(param: Value) -> Result<(), Error> {

    let mut handle = get_tape_handle(&param)?;
    handle.eject_media()?;

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
       },
    },
)]
/// Move to end of media
fn eod(param: Value) -> Result<(), Error> {

    let mut handle = get_tape_handle(&param)?;
    handle.move_to_eom()?;

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
            fast: {
                description: "Use fast erase.",
                type: bool,
                optional: true,
                default: true,
            },
        },
    },
)]
/// Erase media
fn erase(fast: Option<bool>, param: Value) -> Result<(), Error> {

    let mut handle = get_tape_handle(&param)?;
    handle.erase_media(fast.unwrap_or(true))?;

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
            count: {
                description: "File mark count.",
                type: i32,
                minimum: 1
            },
        },
    },
)]
/// Forward space count files (position after file mark).
///
/// The tape is positioned on the first block of the next file.
fn fsf(count: i32, param: Value) -> Result<(), Error> {

    let mut handle = get_tape_handle(&param)?;

    handle.forward_space_count_files(count)?;

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
       },
    },
)]
/// Load media
fn load(param: Value) -> Result<(), Error> {

    let mut handle = get_tape_handle(&param)?;
    handle.mtload()?;

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
       },
    },
)]
/// Rewind the tape
fn rewind(param: Value) -> Result<(), Error> {

    let mut handle = get_tape_handle(&param)?;
    handle.rewind()?;

    Ok(())
}


#[api(
   input: {
        properties: {
           "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        },
    },
)]
/// Scan for existing tape changer devices
fn scan(param: Value) -> Result<(), Error> {

    let output_format = get_output_format(&param);

    let list = linux_tape_device_list();

    if output_format == "json-pretty" {
        println!("{}", serde_json::to_string_pretty(&list)?);
        return Ok(());
    }

    if output_format == "json" {
        println!("{}", serde_json::to_string(&list)?);
        return Ok(());
    }

    if output_format != "text" {
        bail!("unknown output format '{}'", output_format);
    }

    for item in list.iter() {
        println!("{} ({}/{}/{})", item.path, item.vendor, item.model, item.serial);
    }

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
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        },
    },
)]
/// Drive Status
fn status(param: Value) -> Result<(), Error> {

    let output_format = get_output_format(&param);

    let mut handle = get_tape_handle(&param)?;
    let result = handle.get_drive_and_media_status();

    if output_format == "json-pretty" {
        let result = result.map_err(|err: Error| err.to_string());
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }

    if output_format == "json" {
        let result = result.map_err(|err: Error| err.to_string());
        println!("{}", serde_json::to_string(&result)?);
        return Ok(());
    }

    if output_format != "text" {
        bail!("unknown output format '{}'", output_format);
    }

    let status = result?;

    println!("{}", serde_json::to_string_pretty(&status)?);

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
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        },
    },
)]
/// Volume Statistics
fn volume_statistics(param: Value) -> Result<(), Error> {

    let output_format = get_output_format(&param);

    let mut handle = get_tape_handle(&param)?;
    let result = handle.volume_statistics();

    if output_format == "json-pretty" {
        let result = result.map_err(|err: Error| err.to_string());
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }

    if output_format == "json" {
        let result = result.map_err(|err: Error| err.to_string());
        println!("{}", serde_json::to_string(&result)?);
        return Ok(());
    }

    if output_format != "text" {
        bail!("unknown output format '{}'", output_format);
    }

    let data = result?;

    println!("{}", serde_json::to_string_pretty(&data)?);

    Ok(())
}

fn main() -> Result<(), Error> {

    let uid = nix::unistd::Uid::current();

    let username = match nix::unistd::User::from_uid(uid)? {
        Some(user) => user.name,
        None => bail!("unable to get user name"),
    };

    let std_cmd = |method| {
        CliCommand::new(method)
            .completion_cb("drive", complete_drive_name)
            .completion_cb("device", complete_drive_path)
    };

    let cmd_def = CliCommandMap::new()
        .insert("cartridge-memory", std_cmd(&API_METHOD_CARTRIDGE_MEMORY))
        .insert("eject", std_cmd(&API_METHOD_EJECT))
        .insert("eod", std_cmd(&API_METHOD_EOD))
        .insert("erase", std_cmd(&API_METHOD_ERASE))
        .insert("fsf", std_cmd(&API_METHOD_FSF))
        .insert("load", std_cmd(&API_METHOD_LOAD))
        .insert("rewind", std_cmd(&API_METHOD_REWIND))
        .insert("scan", CliCommand::new(&API_METHOD_SCAN))
        .insert("status", std_cmd(&API_METHOD_STATUS))
        .insert("volume-statistics", std_cmd(&API_METHOD_VOLUME_STATISTICS))
        ;

    let mut rpcenv = CliEnvironment::new();
    rpcenv.set_auth_id(Some(format!("{}@pam", username)));

    run_cli_command(cmd_def, rpcenv, None);

    Ok(())
}
