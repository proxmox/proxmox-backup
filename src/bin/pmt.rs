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

use anyhow::{bail, Error};
use serde_json::Value;

use proxmox::{
    api::{
        api,
        cli::*,
        schema::{
            Schema,
            IntegerSchema,
        },
        RpcEnvironment,
    },
};

pub const FILE_MARK_COUNT_SCHEMA: Schema =
    IntegerSchema::new("File mark count.")
    .minimum(1)
    .maximum(i32::MAX as isize)
    .schema();

use proxmox_backup::{
    config::{
        self,
        drive::complete_drive_name,
    },
    api2::types::{
        LINUX_DRIVE_PATH_SCHEMA,
        DRIVE_NAME_SCHEMA,
        LinuxTapeDrive,
    },
    tape::{
        complete_drive_path,
        linux_tape_device_list,
        drive::{
            linux_mtio::MTCmd,
            TapeDriver,
            LinuxTapeHandle,
            open_linux_tape_device,
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
            count: {
                schema: FILE_MARK_COUNT_SCHEMA,
            },
       },
    },
)]
/// Position the tape at the beginning of the count file.
///
/// Positioning is done by first rewinding the tape and then spacing
/// forward over count file marks.
fn asf(count: i32, param: Value) -> Result<(), Error> {

    let mut handle = get_tape_handle(&param)?;

    handle.rewind()?;

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
            count: {
                schema: FILE_MARK_COUNT_SCHEMA,
            },
       },
    },
)]
/// Backward space count files (position before file mark).
///
/// The tape is positioned on the last block of the previous file.
fn bsf(count: i32, param: Value) -> Result<(), Error> {

    let mut handle = get_tape_handle(&param)?;

    handle.backward_space_count_files(count)?;

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
                schema: FILE_MARK_COUNT_SCHEMA,
            },
       },
    },
)]
/// Backward space count files, then forward space one record (position after file mark).
///
/// This leaves the tape positioned at the first block of the file
/// that is count - 1 files before the current file.
fn bsfm(count: i32, param: Value) -> Result<(), Error> {

    let mut handle = get_tape_handle(&param)?;

    handle.mtop(MTCmd::MTBSFM, count, "bsfm")?;

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
                schema: FILE_MARK_COUNT_SCHEMA,
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
            count: {
                schema: FILE_MARK_COUNT_SCHEMA,
            },
        },
    },
)]
/// Forward space count files, then backward space one record (position before file mark).
///
/// This leaves the tape positioned at the last block of the file that
/// is count - 1 files past the current file.
fn fsfm(count: i32, param: Value) -> Result<(), Error> {

    let mut handle = get_tape_handle(&param)?;

    handle.mtop(MTCmd::MTFSFM, count, "fsfm")?;

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
                schema: FILE_MARK_COUNT_SCHEMA,
                optional: true,
             },
        },
    },
)]
/// Write count (default 1) EOF marks at current position.
fn weof(count: Option<i32>, param: Value) -> Result<(), Error> {

    let mut handle = get_tape_handle(&param)?;
    handle.mtop(MTCmd::MTWEOF, count.unwrap_or(1), "write EOF mark")?;

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
        .insert("asf", std_cmd(&API_METHOD_ASF).arg_param(&["count"]))
        .insert("bsf", std_cmd(&API_METHOD_BSF).arg_param(&["count"]))
        .insert("bsfm", std_cmd(&API_METHOD_BSFM).arg_param(&["count"]))
        .insert("cartridge-memory", std_cmd(&API_METHOD_CARTRIDGE_MEMORY))
        .insert("eject", std_cmd(&API_METHOD_EJECT))
        .insert("eod", std_cmd(&API_METHOD_EOD))
        .insert("erase", std_cmd(&API_METHOD_ERASE))
        .insert("fsf", std_cmd(&API_METHOD_FSF).arg_param(&["count"]))
        .insert("fsfm", std_cmd(&API_METHOD_FSFM).arg_param(&["count"]))
        .insert("load", std_cmd(&API_METHOD_LOAD))
        .insert("rewind", std_cmd(&API_METHOD_REWIND))
        .insert("scan", CliCommand::new(&API_METHOD_SCAN))
        .insert("status", std_cmd(&API_METHOD_STATUS))
        .insert("volume-statistics", std_cmd(&API_METHOD_VOLUME_STATISTICS))
        .insert("weof", std_cmd(&API_METHOD_WEOF).arg_param(&["count"]))
        ;

    let mut rpcenv = CliEnvironment::new();
    rpcenv.set_auth_id(Some(format!("{}@pam", username)));

    run_cli_command(cmd_def, rpcenv, None);

    Ok(())
}
