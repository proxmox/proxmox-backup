/// Control magnetic tape drive operation
///
/// This is a Rust implementation, using the Proxmox userspace tape
/// driver. This is meant as a replacement for the 'mt' command line
/// tool.
///
/// Features:
///
/// - written in Rust
/// - use Proxmox userspace driver (using SG_IO)
/// - optional json output format
/// - support tape alert flags
/// - support volume statistics
/// - read cartridge memory
use anyhow::{bail, Error};
use serde_json::Value;

use proxmox_router::cli::*;
use proxmox_router::RpcEnvironment;
use proxmox_schema::{api, ArraySchema, IntegerSchema, Schema, StringSchema};

use pbs_api_types::{LtoTapeDrive, DRIVE_NAME_SCHEMA, LTO_DRIVE_PATH_SCHEMA};
use pbs_config::drive::complete_drive_name;
use pbs_tape::{
    linux_list_drives::{complete_drive_path, lto_tape_device_list, open_lto_tape_device},
    sg_tape::SgTape,
};

pub const FILE_MARK_COUNT_SCHEMA: Schema = IntegerSchema::new("File mark count.")
    .minimum(1)
    .maximum(i32::MAX as isize)
    .schema();

pub const FILE_MARK_POSITION_SCHEMA: Schema = IntegerSchema::new("File mark position (0 is BOT).")
    .minimum(0)
    .maximum(i32::MAX as isize)
    .schema();

pub const RECORD_COUNT_SCHEMA: Schema = IntegerSchema::new("Record count.")
    .minimum(1)
    .maximum(i32::MAX as isize)
    .schema();

pub const DRIVE_OPTION_SCHEMA: Schema =
    StringSchema::new("Lto Tape Driver Option, either numeric value or option name.").schema();

pub const DRIVE_OPTION_LIST_SCHEMA: Schema =
    ArraySchema::new("Drive Option List.", &DRIVE_OPTION_SCHEMA)
        .min_length(1)
        .schema();

fn get_tape_handle(param: &Value) -> Result<SgTape, Error> {
    if let Some(name) = param["drive"].as_str() {
        let (config, _digest) = pbs_config::drive::config()?;
        let drive: LtoTapeDrive = config.lookup("lto", name)?;
        log::info!("using device {}", drive.path);
        return SgTape::new(open_lto_tape_device(&drive.path)?);
    }

    if let Some(device) = param["device"].as_str() {
        log::info!("using device {}", device);
        return SgTape::new(open_lto_tape_device(device)?);
    }

    if let Ok(name) = std::env::var("PROXMOX_TAPE_DRIVE") {
        let (config, _digest) = pbs_config::drive::config()?;
        let drive: LtoTapeDrive = config.lookup("lto", &name)?;
        log::info!("using device {}", drive.path);
        return SgTape::new(open_lto_tape_device(&drive.path)?);
    }

    if let Ok(device) = std::env::var("TAPE") {
        log::info!("using device {}", device);
        return SgTape::new(open_lto_tape_device(&device)?);
    }

    let (config, _digest) = pbs_config::drive::config()?;

    let mut drive_names = Vec::new();
    for (name, (section_type, _)) in config.sections.iter() {
        if section_type != "lto" {
            continue;
        }
        drive_names.push(name);
    }

    if drive_names.len() == 1 {
        let name = drive_names[0];
        let drive: LtoTapeDrive = config.lookup("lto", name)?;
        log::info!("using device {}", drive.path);
        return SgTape::new(open_lto_tape_device(&drive.path)?);
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
                schema: LTO_DRIVE_PATH_SCHEMA,
                optional: true,
            },
            count: {
                schema: FILE_MARK_POSITION_SCHEMA,
            },
       },
    },
)]
/// Position the tape at the beginning of the count file (after
/// filemark count)
fn asf(count: u64, param: Value) -> Result<(), Error> {
    let mut handle = get_tape_handle(&param)?;

    handle.locate_file(count)?;

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
                schema: LTO_DRIVE_PATH_SCHEMA,
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
fn bsf(count: usize, param: Value) -> Result<(), Error> {
    let mut handle = get_tape_handle(&param)?;

    handle.space_filemarks(-count.try_into()?)?;

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
                schema: LTO_DRIVE_PATH_SCHEMA,
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
fn bsfm(count: usize, param: Value) -> Result<(), Error> {
    let mut handle = get_tape_handle(&param)?;

    handle.space_filemarks(-count.try_into()?)?;
    handle.space_filemarks(1)?;

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
                schema: LTO_DRIVE_PATH_SCHEMA,
                optional: true,
            },
            count: {
                schema: RECORD_COUNT_SCHEMA,
            },
        },
    },
)]
/// Backward space records.
fn bsr(count: usize, param: Value) -> Result<(), Error> {
    let mut handle = get_tape_handle(&param)?;

    handle.space_blocks(-count.try_into()?)?;

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
                schema: LTO_DRIVE_PATH_SCHEMA,
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
                schema: LTO_DRIVE_PATH_SCHEMA,
                optional: true,
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        },
    },
)]
/// Read Tape Alert Flags
fn tape_alert_flags(param: Value) -> Result<(), Error> {
    let output_format = get_output_format(&param);

    let mut handle = get_tape_handle(&param)?;
    let result = handle
        .tape_alert_flags()
        .map(|flags| format!("{:?}", flags));

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

    let flags = result?;
    println!("Tape Alert Flags: {}", flags);

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
                schema: LTO_DRIVE_PATH_SCHEMA,
                optional: true,
            },
       },
    },
)]
/// Eject drive media
fn eject(param: Value) -> Result<(), Error> {
    let mut handle = get_tape_handle(&param)?;
    handle.eject()?;

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
                schema: LTO_DRIVE_PATH_SCHEMA,
                optional: true,
            },
       },
    },
)]
/// Move to end of media
fn eod(param: Value) -> Result<(), Error> {
    let mut handle = get_tape_handle(&param)?;
    handle.move_to_eom(false)?;

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
                schema: LTO_DRIVE_PATH_SCHEMA,
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
/// Erase media (from current position)
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
                schema: LTO_DRIVE_PATH_SCHEMA,
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
/// Format media,  single partition
fn format(fast: Option<bool>, param: Value) -> Result<(), Error> {
    let mut handle = get_tape_handle(&param)?;
    handle.format_media(fast.unwrap_or(true))?;

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
                schema: LTO_DRIVE_PATH_SCHEMA,
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
fn fsf(count: usize, param: Value) -> Result<(), Error> {
    let mut handle = get_tape_handle(&param)?;

    handle.space_filemarks(count.try_into()?)?;

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
                schema: LTO_DRIVE_PATH_SCHEMA,
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
fn fsfm(count: usize, param: Value) -> Result<(), Error> {
    let mut handle = get_tape_handle(&param)?;

    handle.space_filemarks(count.try_into()?)?;
    handle.space_filemarks(-1)?;

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
                schema: LTO_DRIVE_PATH_SCHEMA,
                optional: true,
            },
            count: {
                schema: RECORD_COUNT_SCHEMA,
            },
        },
    },
)]
/// Forward space records.
fn fsr(count: usize, param: Value) -> Result<(), Error> {
    let mut handle = get_tape_handle(&param)?;

    handle.space_blocks(count.try_into()?)?;

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
                schema: LTO_DRIVE_PATH_SCHEMA,
                optional: true,
            },
       },
    },
)]
/// Load media
fn load(param: Value) -> Result<(), Error> {
    let mut handle = get_tape_handle(&param)?;
    handle.load()?;

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
                schema: LTO_DRIVE_PATH_SCHEMA,
                optional: true,
            },
       },
    },
)]
/// Lock the tape drive door
fn lock(param: Value) -> Result<(), Error> {
    let mut handle = get_tape_handle(&param)?;

    handle.set_medium_removal(false)?;

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
                schema: LTO_DRIVE_PATH_SCHEMA,
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

    let list = lto_tape_device_list();

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
        println!(
            "{} ({}/{}/{})",
            item.path, item.vendor, item.model, item.serial
        );
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
                schema: LTO_DRIVE_PATH_SCHEMA,
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
                schema: LTO_DRIVE_PATH_SCHEMA,
                optional: true,
            },
       },
    },
)]
/// Unlock the tape drive door
fn unlock(param: Value) -> Result<(), Error> {
    let mut handle = get_tape_handle(&param)?;

    handle.set_medium_removal(true)?;

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
                schema: LTO_DRIVE_PATH_SCHEMA,
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
                schema: LTO_DRIVE_PATH_SCHEMA,
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
fn weof(count: Option<usize>, param: Value) -> Result<(), Error> {
    let count = count.unwrap_or(1);

    let mut handle = get_tape_handle(&param)?;

    handle.write_filemarks(count, false)?;

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
                schema: LTO_DRIVE_PATH_SCHEMA,
                optional: true,
            },
            compression: {
                description: "Enable/disable compression.",
                type: bool,
                optional: true,
            },
            blocksize: {
                description: "Set tape drive block_length (0 is variable length).",
                type: u32,
                minimum: 0,
                maximum: 0x80_00_00,
                optional: true,
            },
            buffer_mode: {
                description: "Use drive buffer.",
                type: bool,
                optional: true,
            },
            defaults: {
                description: "Set default options",
                type: bool,
                optional: true,
            },
        },
    },
)]
/// Set varios drive options
fn options(
    compression: Option<bool>,
    blocksize: Option<u32>,
    buffer_mode: Option<bool>,
    defaults: Option<bool>,
    param: Value,
) -> Result<(), Error> {
    let mut handle = get_tape_handle(&param)?;

    if let Some(true) = defaults {
        handle.set_default_options()?;
    }

    handle.set_drive_options(compression, blocksize, buffer_mode)?;

    Ok(())
}

fn main() -> Result<(), Error> {
    init_cli_logger("PBS_LOG", "info");

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
        .usage_skip_options(&["device", "drive", "output-format"])
        .insert("asf", std_cmd(&API_METHOD_ASF).arg_param(&["count"]))
        .insert("bsf", std_cmd(&API_METHOD_BSF).arg_param(&["count"]))
        .insert("bsfm", std_cmd(&API_METHOD_BSFM).arg_param(&["count"]))
        .insert("bsr", std_cmd(&API_METHOD_BSR).arg_param(&["count"]))
        .insert("cartridge-memory", std_cmd(&API_METHOD_CARTRIDGE_MEMORY))
        .insert("eject", std_cmd(&API_METHOD_EJECT))
        .insert("eod", std_cmd(&API_METHOD_EOD))
        .insert("erase", std_cmd(&API_METHOD_ERASE))
        .insert("format", std_cmd(&API_METHOD_FORMAT))
        .insert("fsf", std_cmd(&API_METHOD_FSF).arg_param(&["count"]))
        .insert("fsfm", std_cmd(&API_METHOD_FSFM).arg_param(&["count"]))
        .insert("fsr", std_cmd(&API_METHOD_FSR).arg_param(&["count"]))
        .insert("load", std_cmd(&API_METHOD_LOAD))
        .insert("lock", std_cmd(&API_METHOD_LOCK))
        .insert("options", std_cmd(&API_METHOD_OPTIONS))
        .insert("rewind", std_cmd(&API_METHOD_REWIND))
        .insert("scan", CliCommand::new(&API_METHOD_SCAN))
        .insert("status", std_cmd(&API_METHOD_STATUS))
        .insert("tape-alert-flags", std_cmd(&API_METHOD_TAPE_ALERT_FLAGS))
        .insert("unlock", std_cmd(&API_METHOD_UNLOCK))
        .insert("volume-statistics", std_cmd(&API_METHOD_VOLUME_STATISTICS))
        .insert("weof", std_cmd(&API_METHOD_WEOF).arg_param(&["count"]));

    let mut rpcenv = CliEnvironment::new();
    rpcenv.set_auth_id(Some(format!("{}@pam", username)));

    run_cli_command(cmd_def, rpcenv, None);

    Ok(())
}
