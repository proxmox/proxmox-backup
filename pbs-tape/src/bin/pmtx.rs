/// SCSI changer command implemented using scsi-generic raw commands
///
/// This is a Rust implementation, meant to replace the 'mtx' command
/// line tool.
///
/// Features:
///
/// - written in Rust
///
/// - json output
///
/// - list serial number for attached drives, so that it is possible
///   to associate drive numbers with drives.
use std::fs::File;

use anyhow::{bail, Error};
use serde_json::Value;

use proxmox_router::cli::*;
use proxmox_router::RpcEnvironment;
use proxmox_schema::api;

use pbs_api_types::{LtoTapeDrive, ScsiTapeChanger, CHANGER_NAME_SCHEMA, SCSI_CHANGER_PATH_SCHEMA};
use pbs_config::drive::complete_changer_name;
use pbs_tape::{
    linux_list_drives::{complete_changer_path, linux_tape_changer_list},
    sg_pt_changer,
    sgutils2::scsi_inquiry,
    ElementStatus,
};

fn get_changer_handle(param: &Value) -> Result<File, Error> {
    if let Some(name) = param["changer"].as_str() {
        let (config, _digest) = pbs_config::drive::config()?;
        let changer_config: ScsiTapeChanger = config.lookup("changer", name)?;
        log::info!("using device {}", changer_config.path);
        return sg_pt_changer::open(&changer_config.path);
    }

    if let Some(device) = param["device"].as_str() {
        log::info!("using device {}", device);
        return sg_pt_changer::open(device);
    }

    if let Ok(name) = std::env::var("PROXMOX_TAPE_DRIVE") {
        let (config, _digest) = pbs_config::drive::config()?;
        let drive: LtoTapeDrive = config.lookup("lto", &name)?;
        if let Some(changer) = drive.changer {
            let changer_config: ScsiTapeChanger = config.lookup("changer", &changer)?;
            log::info!("using device {}", changer_config.path);
            return sg_pt_changer::open(&changer_config.path);
        }
    }

    if let Ok(device) = std::env::var("CHANGER") {
        log::info!("using device {}", device);
        return sg_pt_changer::open(device);
    }

    bail!("no  changer device specified");
}

#[api(
   input: {
        properties: {
            changer: {
                schema: CHANGER_NAME_SCHEMA,
                optional: true,
            },
            device: {
                schema: SCSI_CHANGER_PATH_SCHEMA,
                optional: true,
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
       },
    },
)]
/// Inquiry
fn inquiry(param: Value) -> Result<(), Error> {
    let output_format = get_output_format(&param);

    let result: Result<_, Error> = proxmox_lang::try_block!({
        let mut file = get_changer_handle(&param)?;
        let info = scsi_inquiry(&mut file)?;
        Ok(info)
    });

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

    let info = result?;

    println!(
        "Type:     {} ({})",
        info.peripheral_type_text, info.peripheral_type
    );
    println!("Vendor:   {}", info.vendor);
    println!("Product:  {}", info.product);
    println!("Revision: {}", info.revision);

    Ok(())
}

#[api(
   input: {
        properties: {
            changer: {
                schema: CHANGER_NAME_SCHEMA,
                optional: true,
            },
            device: {
                schema: SCSI_CHANGER_PATH_SCHEMA,
                optional: true,
            },
        },
    },
)]
/// Inventory
fn inventory(param: Value) -> Result<(), Error> {
    let mut file = get_changer_handle(&param)?;
    sg_pt_changer::initialize_element_status(&mut file)?;

    Ok(())
}

#[api(
   input: {
        properties: {
            changer: {
                schema: CHANGER_NAME_SCHEMA,
                optional: true,
            },
            device: {
                schema: SCSI_CHANGER_PATH_SCHEMA,
                optional: true,
            },
            slot: {
                description: "Storage slot number (source).",
                type: u64,
            },
            drivenum: {
                description: "Target drive number (defaults to Drive 0)",
                type: u64,
                optional: true,
            },
        },
    },
)]
/// Load
fn load(param: Value, slot: u64, drivenum: Option<u64>) -> Result<(), Error> {
    let mut file = get_changer_handle(&param)?;

    let drivenum = drivenum.unwrap_or(0);

    sg_pt_changer::load_slot(&mut file, slot, drivenum)?;

    Ok(())
}

#[api(
   input: {
        properties: {
            changer: {
                schema: CHANGER_NAME_SCHEMA,
                optional: true,
            },
            device: {
                schema: SCSI_CHANGER_PATH_SCHEMA,
                optional: true,
            },
            slot: {
                description: "Storage slot number (target). If omitted, defaults to the slot that the drive was loaded from.",
                type: u64,
                optional: true,
            },
            drivenum: {
                description: "Target drive number (defaults to Drive 0)",
                type: u64,
                optional: true,
            },
        },
    },
)]
/// Unload
fn unload(param: Value, slot: Option<u64>, drivenum: Option<u64>) -> Result<(), Error> {
    let mut file = get_changer_handle(&param)?;

    let drivenum = drivenum.unwrap_or(0);

    if let Some(to_slot) = slot {
        sg_pt_changer::unload(&mut file, to_slot, drivenum)?;
        return Ok(());
    }

    let status = sg_pt_changer::read_element_status(&mut file)?;

    if let Some(info) = status.drives.get(drivenum as usize) {
        if let ElementStatus::Empty = info.status {
            bail!("Drive {} is empty.", drivenum);
        }
        if let Some(to_slot) = info.loaded_slot {
            // check if original slot is empty/usable
            if let Some(slot_info) = status.slots.get(to_slot as usize - 1) {
                if let ElementStatus::Empty = slot_info.status {
                    sg_pt_changer::unload(&mut file, to_slot, drivenum)?;
                    return Ok(());
                }
            }
        }

        if let Some(to_slot) = status.find_free_slot(false) {
            sg_pt_changer::unload(&mut file, to_slot, drivenum)?;
            Ok(())
        } else {
            bail!("Drive '{}' unload failure - no free slot", drivenum);
        }
    } else {
        bail!("Drive {} does not exist.", drivenum);
    }
}

#[api(
   input: {
        properties: {
            changer: {
                schema: CHANGER_NAME_SCHEMA,
                optional: true,
            },
            device: {
                schema: SCSI_CHANGER_PATH_SCHEMA,
                optional: true,
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
       },
    },
)]
/// Changer Status
fn status(param: Value) -> Result<(), Error> {
    let output_format = get_output_format(&param);

    let result: Result<_, Error> = proxmox_lang::try_block!({
        let mut file = get_changer_handle(&param)?;
        let status = sg_pt_changer::read_element_status(&mut file)?;
        Ok(status)
    });

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

    for (i, transport) in status.transports.iter().enumerate() {
        println!(
            "Transport Element (Griper)    {:>3}: {:?}",
            i, transport.status
        );
    }

    for (i, drive) in status.drives.iter().enumerate() {
        let loaded_txt = match drive.loaded_slot {
            Some(slot) => format!(", Source: {}", slot),
            None => String::new(),
        };
        let serial_txt = match drive.drive_serial_number {
            Some(ref serial) => format!(", Serial: {}", serial),
            None => String::new(),
        };

        println!(
            "Data Transfer Element (Drive) {:>3}: {:?}{}{}",
            i, drive.status, loaded_txt, serial_txt,
        );
    }

    for (i, slot) in status.slots.iter().enumerate() {
        if slot.import_export {
            println!("  Import/Export   {:>3}: {:?}", i + 1, slot.status);
        } else {
            println!("  Storage Element {:>3}: {:?}", i + 1, slot.status);
        }
    }

    Ok(())
}

#[api(
   input: {
        properties: {
            changer: {
                schema: CHANGER_NAME_SCHEMA,
                optional: true,
            },
            device: {
                schema: SCSI_CHANGER_PATH_SCHEMA,
                optional: true,
            },
            from: {
                description: "Source storage slot number.",
                type: u64,
            },
            to: {
                description: "Target storage slot number.",
                type: u64,
            },
        },
    },
)]
/// Transfer
fn transfer(param: Value, from: u64, to: u64) -> Result<(), Error> {
    let mut file = get_changer_handle(&param)?;

    sg_pt_changer::transfer_medium(&mut file, from, to)?;

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

    let list = linux_tape_changer_list();

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

fn main() -> Result<(), Error> {
    init_cli_logger("PBS_LOG", "info");

    let uid = nix::unistd::Uid::current();

    let username = match nix::unistd::User::from_uid(uid)? {
        Some(user) => user.name,
        None => bail!("unable to get user name"),
    };

    let cmd_def = CliCommandMap::new()
        .usage_skip_options(&["device", "changer", "output-format"])
        .insert(
            "inquiry",
            CliCommand::new(&API_METHOD_INQUIRY)
                .completion_cb("changer", complete_changer_name)
                .completion_cb("device", complete_changer_path),
        )
        .insert(
            "inventory",
            CliCommand::new(&API_METHOD_INVENTORY)
                .completion_cb("changer", complete_changer_name)
                .completion_cb("device", complete_changer_path),
        )
        .insert(
            "load",
            CliCommand::new(&API_METHOD_LOAD)
                .arg_param(&["slot"])
                .completion_cb("changer", complete_changer_name)
                .completion_cb("device", complete_changer_path),
        )
        .insert(
            "unload",
            CliCommand::new(&API_METHOD_UNLOAD)
                .completion_cb("changer", complete_changer_name)
                .completion_cb("device", complete_changer_path),
        )
        .insert("scan", CliCommand::new(&API_METHOD_SCAN))
        .insert(
            "status",
            CliCommand::new(&API_METHOD_STATUS)
                .completion_cb("changer", complete_changer_name)
                .completion_cb("device", complete_changer_path),
        )
        .insert(
            "transfer",
            CliCommand::new(&API_METHOD_TRANSFER)
                .arg_param(&["from", "to"])
                .completion_cb("changer", complete_changer_name)
                .completion_cb("device", complete_changer_path),
        );

    let mut rpcenv = CliEnvironment::new();
    rpcenv.set_auth_id(Some(format!("{}@pam", username)));

    run_cli_command(cmd_def, rpcenv, None);

    Ok(())
}
