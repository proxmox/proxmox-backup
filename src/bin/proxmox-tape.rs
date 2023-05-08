use std::collections::HashMap;

use anyhow::{bail, format_err, Error};
use serde_json::{json, Value};

use proxmox_human_byte::HumanByte;
use proxmox_io::ReadExt;
use proxmox_router::cli::*;
use proxmox_router::RpcEnvironment;
use proxmox_schema::api;
use proxmox_section_config::SectionConfigData;
use proxmox_time::strftime_local;

use pbs_client::view_task_result;
use pbs_tools::format::{render_bytes_human_readable, render_epoch};

use pbs_config::datastore::complete_datastore_name;
use pbs_config::drive::complete_drive_name;
use pbs_config::media_pool::complete_pool_name;

use pbs_api_types::{
    Authid, BackupNamespace, GroupListItem, Userid, DATASTORE_MAP_LIST_SCHEMA, DATASTORE_SCHEMA,
    DRIVE_NAME_SCHEMA, GROUP_FILTER_LIST_SCHEMA, MEDIA_LABEL_SCHEMA, MEDIA_POOL_NAME_SCHEMA,
    NS_MAX_DEPTH_SCHEMA, TAPE_RESTORE_NAMESPACE_SCHEMA, TAPE_RESTORE_SNAPSHOT_SCHEMA,
};
use pbs_tape::{BlockReadError, MediaContentHeader, PROXMOX_BACKUP_CONTENT_HEADER_MAGIC_1_0};

use proxmox_backup::{
    api2,
    client_helpers::connect_to_localhost,
    tape::{
        complete_media_label_text, complete_media_set_snapshots, complete_media_set_uuid,
        drive::{lock_tape_device, open_drive, set_tape_device_state},
        file_formats::proxmox_tape_magic_to_text,
    },
};

mod proxmox_tape;
use proxmox_tape::*;

async fn get_backup_groups(store: &str) -> Result<Vec<GroupListItem>, Error> {
    let client = connect_to_localhost()?;
    let api_res = client
        .get(&format!("api2/json/admin/datastore/{}/groups", store), None)
        .await?;

    match api_res.get("data") {
        Some(data) => Ok(serde_json::from_value::<Vec<GroupListItem>>(
            data.to_owned(),
        )?),
        None => bail!("could not get group list"),
    }
}

// shell completion helper
pub fn complete_datastore_group_filter(_arg: &str, param: &HashMap<String, String>) -> Vec<String> {
    let mut list = vec![
        "regex:".to_string(),
        "type:ct".to_string(),
        "type:host".to_string(),
        "type:vm".to_string(),
    ];

    if let Some(store) = param.get("store") {
        let groups = proxmox_async::runtime::block_on(async { get_backup_groups(store).await });
        if let Ok(groups) = groups {
            list.extend(
                groups
                    .iter()
                    .map(|group| format!("group:{}/{}", group.backup.ty, group.backup.id)),
            );
        }
    }

    list
}

pub fn extract_drive_name(param: &mut Value, config: &SectionConfigData) -> Result<String, Error> {
    let drive = param["drive"]
        .as_str()
        .map(String::from)
        .or_else(|| std::env::var("PROXMOX_TAPE_DRIVE").ok())
        .or_else(|| {
            let mut drive_names = Vec::new();

            for (name, (section_type, _)) in config.sections.iter() {
                if section_type == "linux" || section_type == "virtual" {
                    drive_names.push(name);
                }
            }

            if drive_names.len() == 1 {
                Some(drive_names[0].to_owned())
            } else {
                None
            }
        })
        .ok_or_else(|| format_err!("unable to get (default) drive name"))?;

    if let Some(map) = param.as_object_mut() {
        map.remove("drive");
    }

    Ok(drive)
}

#[api(
    input: {
        properties: {
            drive: {
                schema: DRIVE_NAME_SCHEMA,
                optional: true,
            },
            fast: {
                description: "Use fast erase.",
                type: bool,
                optional: true,
                default: true,
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
       },
    },
)]
/// Format media
async fn format_media(mut param: Value) -> Result<(), Error> {
    let output_format = extract_output_format(&mut param);

    let (config, _digest) = pbs_config::drive::config()?;

    let drive = extract_drive_name(&mut param, &config)?;

    let client = connect_to_localhost()?;

    let path = format!("api2/json/tape/drive/{}/format-media", drive);
    let result = client.post(&path, Some(param)).await?;

    view_task_result(&client, result, &output_format).await?;

    Ok(())
}

#[api(
    input: {
        properties: {
            drive: {
                schema: DRIVE_NAME_SCHEMA,
                optional: true,
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        },
    },
)]
/// Rewind tape
async fn rewind(mut param: Value) -> Result<(), Error> {
    let output_format = extract_output_format(&mut param);

    let (config, _digest) = pbs_config::drive::config()?;

    let drive = extract_drive_name(&mut param, &config)?;

    let client = connect_to_localhost()?;

    let path = format!("api2/json/tape/drive/{}/rewind", drive);
    let result = client.post(&path, Some(param)).await?;

    view_task_result(&client, result, &output_format).await?;

    Ok(())
}

#[api(
    input: {
        properties: {
            drive: {
                schema: DRIVE_NAME_SCHEMA,
                optional: true,
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        },
    },
)]
/// Eject/Unload drive media
async fn eject_media(mut param: Value) -> Result<(), Error> {
    let output_format = extract_output_format(&mut param);

    let (config, _digest) = pbs_config::drive::config()?;

    let drive = extract_drive_name(&mut param, &config)?;

    let client = connect_to_localhost()?;

    let path = format!("api2/json/tape/drive/{}/eject-media", drive);
    let result = client.post(&path, Some(param)).await?;

    view_task_result(&client, result, &output_format).await?;

    Ok(())
}

#[api(
    input: {
        properties: {
            drive: {
                schema: DRIVE_NAME_SCHEMA,
                optional: true,
            },
            "label-text": {
                schema: MEDIA_LABEL_SCHEMA,
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        },
    },
)]
/// Load media with specified label
async fn load_media(mut param: Value) -> Result<(), Error> {
    let output_format = extract_output_format(&mut param);

    let (config, _digest) = pbs_config::drive::config()?;

    let drive = extract_drive_name(&mut param, &config)?;

    let client = connect_to_localhost()?;

    let path = format!("api2/json/tape/drive/{}/load-media", drive);
    let result = client.post(&path, Some(param)).await?;

    view_task_result(&client, result, &output_format).await?;

    Ok(())
}

#[api(
    input: {
        properties: {
            drive: {
                schema: DRIVE_NAME_SCHEMA,
                optional: true,
            },
            "label-text": {
                schema: MEDIA_LABEL_SCHEMA,
            },
        },
    },
)]
/// Export media with specified label
async fn export_media(mut param: Value) -> Result<(), Error> {
    let (config, _digest) = pbs_config::drive::config()?;

    let drive = extract_drive_name(&mut param, &config)?;

    let client = connect_to_localhost()?;

    let path = format!("api2/json/tape/drive/{}/export-media", drive);
    client.put(&path, Some(param)).await?;

    Ok(())
}

#[api(
    input: {
        properties: {
            drive: {
                schema: DRIVE_NAME_SCHEMA,
                optional: true,
            },
            "source-slot": {
                description: "Source slot number.",
                type: u64,
                minimum: 1,
            },
        },
    },
)]
/// Load media from the specified slot
async fn load_media_from_slot(mut param: Value) -> Result<(), Error> {
    let (config, _digest) = pbs_config::drive::config()?;

    let drive = extract_drive_name(&mut param, &config)?;

    let client = connect_to_localhost()?;

    let path = format!("api2/json/tape/drive/{}/load-slot", drive);
    client.post(&path, Some(param)).await?;

    Ok(())
}

#[api(
    input: {
        properties: {
            drive: {
                schema: DRIVE_NAME_SCHEMA,
                optional: true,
            },
            "target-slot": {
                description: "Target slot number. If omitted, defaults to the slot that the drive was loaded from.",
                type: u64,
                minimum: 1,
                optional: true,
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        },
    },
)]
/// Unload media via changer
async fn unload_media(mut param: Value) -> Result<(), Error> {
    let output_format = extract_output_format(&mut param);

    let (config, _digest) = pbs_config::drive::config()?;

    let drive = extract_drive_name(&mut param, &config)?;

    let client = connect_to_localhost()?;

    let path = format!("api2/json/tape/drive/{}/unload", drive);
    let result = client.post(&path, Some(param)).await?;

    view_task_result(&client, result, &output_format).await?;

    Ok(())
}

#[api(
    input: {
        properties: {
            pool: {
                schema: MEDIA_POOL_NAME_SCHEMA,
                optional: true,
            },
            drive: {
                schema: DRIVE_NAME_SCHEMA,
                optional: true,
            },
            "label-text": {
                schema: MEDIA_LABEL_SCHEMA,
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        },
    },
)]
/// Label media
async fn label_media(mut param: Value) -> Result<(), Error> {
    let output_format = extract_output_format(&mut param);

    let (config, _digest) = pbs_config::drive::config()?;

    let drive = extract_drive_name(&mut param, &config)?;

    let client = connect_to_localhost()?;

    let path = format!("api2/json/tape/drive/{}/label-media", drive);
    let result = client.post(&path, Some(param)).await?;

    view_task_result(&client, result, &output_format).await?;

    Ok(())
}

#[api(
    input: {
        properties: {
            drive: {
                schema: DRIVE_NAME_SCHEMA,
                optional: true,
            },
            inventorize: {
                description: "Inventorize media",
                type: bool,
                optional: true,
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
             },
        },
    },
)]
/// Read media label
async fn read_label(mut param: Value) -> Result<(), Error> {
    let output_format = extract_output_format(&mut param);

    let (config, _digest) = pbs_config::drive::config()?;

    let drive = extract_drive_name(&mut param, &config)?;

    let client = connect_to_localhost()?;

    let path = format!("api2/json/tape/drive/{}/read-label", drive);
    let mut result = client.get(&path, Some(param)).await?;
    let mut data = result["data"].take();

    let info = &api2::tape::drive::API_METHOD_READ_LABEL;

    let options = default_table_format_options()
        .column(ColumnConfig::new("label-text"))
        .column(ColumnConfig::new("uuid"))
        .column(ColumnConfig::new("ctime").renderer(render_epoch))
        .column(ColumnConfig::new("pool"))
        .column(ColumnConfig::new("media-set-uuid"))
        .column(ColumnConfig::new("media-set-ctime").renderer(render_epoch))
        .column(ColumnConfig::new("encryption-key-fingerprint"));

    format_and_print_result_full(&mut data, &info.returns, &output_format, &options);

    Ok(())
}

#[api(
    input: {
        properties: {
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
            drive: {
                schema: DRIVE_NAME_SCHEMA,
                optional: true,
            },
            "read-labels": {
                description: "Load unknown tapes and try read labels",
                type: bool,
                optional: true,
                default: false,
            },
            "read-all-labels": {
                description: "Load all tapes and try read labels (even if already inventoried)",
                type: bool,
                optional: true,
                default: false,
            },
            "catalog": {
                description: "Try to restore catalogs from tapes.",
                type: bool,
                default: false,
                optional: true,
            }
        },
    },
)]
/// List (and update) media labels (Changer Inventory)
async fn inventory(
    read_labels: bool,
    read_all_labels: bool,
    catalog: bool,
    mut param: Value,
) -> Result<(), Error> {
    let output_format = extract_output_format(&mut param);

    let (config, _digest) = pbs_config::drive::config()?;
    let drive = extract_drive_name(&mut param, &config)?;

    let do_read = read_labels || read_all_labels || catalog;

    let client = connect_to_localhost()?;

    let path = format!("api2/json/tape/drive/{}/inventory", drive);

    if do_read {
        let mut param = json!({});
        param["read-all-labels"] = read_all_labels.into();
        param["catalog"] = catalog.into();

        let result = client.put(&path, Some(param)).await?; // update inventory
        view_task_result(&client, result, &output_format).await?;
    }

    let mut result = client.get(&path, None).await?;
    let mut data = result["data"].take();

    let info = &api2::tape::drive::API_METHOD_INVENTORY;

    let options = default_table_format_options()
        .column(ColumnConfig::new("label-text"))
        .column(ColumnConfig::new("uuid"));

    format_and_print_result_full(&mut data, &info.returns, &output_format, &options);

    Ok(())
}

#[api(
    input: {
        properties: {
            pool: {
                schema: MEDIA_POOL_NAME_SCHEMA,
                optional: true,
            },
            drive: {
                schema: DRIVE_NAME_SCHEMA,
                optional: true,
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        },
    },
)]
/// Label media with barcodes from changer device
async fn barcode_label_media(mut param: Value) -> Result<(), Error> {
    let output_format = extract_output_format(&mut param);

    let (config, _digest) = pbs_config::drive::config()?;

    let drive = extract_drive_name(&mut param, &config)?;

    let client = connect_to_localhost()?;

    let path = format!("api2/json/tape/drive/{}/barcode-label-media", drive);
    let result = client.post(&path, Some(param)).await?;

    view_task_result(&client, result, &output_format).await?;

    Ok(())
}

#[api(
    input: {
        properties: {
            drive: {
                schema: DRIVE_NAME_SCHEMA,
                optional: true,
            },
        },
    },
)]
/// Move to end of media (MTEOM, used to debug)
fn move_to_eom(mut param: Value) -> Result<(), Error> {
    let (config, _digest) = pbs_config::drive::config()?;

    let drive = extract_drive_name(&mut param, &config)?;

    let _lock = lock_tape_device(&config, &drive)?;
    set_tape_device_state(&drive, "moving to eom")?;

    let mut drive = open_drive(&config, &drive)?;

    drive.move_to_eom(false)?;

    Ok(())
}

#[api(
    input: {
        properties: {
            drive: {
                schema: DRIVE_NAME_SCHEMA,
                optional: true,
            },
        },
    },
)]
/// Rewind, then read media contents and print debug info
///
/// Note: This reads unless the driver returns an IO Error, so this
/// method is expected to fails when we reach EOT.
fn debug_scan(mut param: Value) -> Result<(), Error> {
    let (config, _digest) = pbs_config::drive::config()?;

    let drive = extract_drive_name(&mut param, &config)?;

    let _lock = lock_tape_device(&config, &drive)?;
    set_tape_device_state(&drive, "debug scan")?;

    let mut drive = open_drive(&config, &drive)?;

    println!("rewinding tape");
    drive.rewind()?;

    loop {
        let file_number = drive.current_file_number()?;

        match drive.read_next_file() {
            Err(BlockReadError::EndOfFile) => {
                println!("filemark number {}", file_number);
                continue;
            }
            Err(BlockReadError::EndOfStream) => {
                println!("got EOT");
                return Ok(());
            }
            Err(BlockReadError::Error(err)) => {
                return Err(err.into());
            }
            Ok(mut reader) => {
                println!("got file number {}", file_number);

                let header: Result<MediaContentHeader, _> = unsafe { reader.read_le_value() };
                match header {
                    Ok(header) => {
                        if header.magic != PROXMOX_BACKUP_CONTENT_HEADER_MAGIC_1_0 {
                            println!(
                                "got MediaContentHeader with wrong magic: {:?}",
                                header.magic
                            );
                        } else if let Some(name) = proxmox_tape_magic_to_text(&header.content_magic)
                        {
                            println!("got content header: {}", name);
                            println!("  uuid:  {}", header.content_uuid());
                            println!("  ctime: {}", strftime_local("%c", header.ctime)?);
                            println!("  hsize: {}", HumanByte::from(header.size as usize));
                            println!("  part:  {}", header.part_number);
                        } else {
                            println!("got unknown content header: {:?}", header.content_magic);
                        }
                    }
                    Err(err) => {
                        println!("unable to read content header - {}", err);
                    }
                }
                let bytes = reader.skip_data()?;
                println!("skipped {}", HumanByte::from(bytes));
                if let Ok(true) = reader.has_end_marker() {
                    if reader.is_incomplete()? {
                        println!("WARNING: file is incomplete");
                    }
                } else {
                    println!("WARNING: file without end marker");
                }
            }
        }
    }
}

#[api(
    input: {
        properties: {
            drive: {
                schema: DRIVE_NAME_SCHEMA,
                optional: true,
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        },
    },
)]
/// Read Cartridge Memory (Medium auxiliary memory attributes)
async fn cartridge_memory(mut param: Value) -> Result<(), Error> {
    let output_format = extract_output_format(&mut param);

    let (config, _digest) = pbs_config::drive::config()?;

    let drive = extract_drive_name(&mut param, &config)?;

    let client = connect_to_localhost()?;

    let path = format!("api2/json/tape/drive/{}/cartridge-memory", drive);
    let mut result = client.get(&path, Some(param)).await?;
    let mut data = result["data"].take();

    let info = &api2::tape::drive::API_METHOD_CARTRIDGE_MEMORY;

    let options = default_table_format_options()
        .column(ColumnConfig::new("id"))
        .column(ColumnConfig::new("name"))
        .column(ColumnConfig::new("value"));

    format_and_print_result_full(&mut data, &info.returns, &output_format, &options);
    Ok(())
}

#[api(
    input: {
        properties: {
            drive: {
                schema: DRIVE_NAME_SCHEMA,
                optional: true,
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        },
    },
)]
/// Read Volume Statistics (SCSI log page 17h)
async fn volume_statistics(mut param: Value) -> Result<(), Error> {
    let output_format = extract_output_format(&mut param);

    let (config, _digest) = pbs_config::drive::config()?;

    let drive = extract_drive_name(&mut param, &config)?;

    let client = connect_to_localhost()?;

    let path = format!("api2/json/tape/drive/{}/volume-statistics", drive);
    let mut result = client.get(&path, Some(param)).await?;
    let mut data = result["data"].take();

    let info = &api2::tape::drive::API_METHOD_VOLUME_STATISTICS;

    let options = default_table_format_options();

    format_and_print_result_full(&mut data, &info.returns, &output_format, &options);

    Ok(())
}

#[api(
    input: {
        properties: {
            drive: {
                schema: DRIVE_NAME_SCHEMA,
                optional: true,
            },
             "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
             },
        },
    },
)]
/// Get drive/media status
async fn status(mut param: Value) -> Result<(), Error> {
    let output_format = extract_output_format(&mut param);

    let (config, _digest) = pbs_config::drive::config()?;

    let drive = extract_drive_name(&mut param, &config)?;

    let client = connect_to_localhost()?;

    let path = format!("api2/json/tape/drive/{}/status", drive);
    let mut result = client.get(&path, Some(param)).await?;
    let mut data = result["data"].take();

    let info = &api2::tape::drive::API_METHOD_STATUS;

    let render_percentage = |value: &Value, _record: &Value| {
        match value.as_f64() {
            Some(wearout) => Ok(format!("{:.2}%", wearout * 100.0)),
            None => Ok(String::from("ERROR")), // should never happen
        }
    };

    let options = default_table_format_options()
        .column(ColumnConfig::new("blocksize"))
        .column(ColumnConfig::new("density"))
        .column(ColumnConfig::new("compression"))
        .column(ColumnConfig::new("buffer-mode"))
        .column(ColumnConfig::new("write-protect"))
        .column(ColumnConfig::new("alert-flags"))
        .column(ColumnConfig::new("file-number"))
        .column(ColumnConfig::new("block-number"))
        .column(ColumnConfig::new("manufactured").renderer(render_epoch))
        .column(ColumnConfig::new("bytes-written").renderer(render_bytes_human_readable))
        .column(ColumnConfig::new("bytes-read").renderer(render_bytes_human_readable))
        .column(ColumnConfig::new("medium-passes"))
        .column(ColumnConfig::new("medium-wearout").renderer(render_percentage))
        .column(ColumnConfig::new("volume-mounts"));

    format_and_print_result_full(&mut data, &info.returns, &output_format, &options);

    Ok(())
}

#[api(
    input: {
        properties: {
            drive: {
                schema: DRIVE_NAME_SCHEMA,
                optional: true,
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        },
    },
)]
/// Clean drive
async fn clean_drive(mut param: Value) -> Result<(), Error> {
    let output_format = extract_output_format(&mut param);

    let (config, _digest) = pbs_config::drive::config()?;

    let drive = extract_drive_name(&mut param, &config)?;

    let client = connect_to_localhost()?;

    let path = format!("api2/json/tape/drive/{}/clean", drive);
    let result = client.put(&path, Some(param)).await?;

    view_task_result(&client, result, &output_format).await?;

    Ok(())
}

#[api(
    input: {
        properties: {

            // Note: We cannot use TapeBackupJobSetup, because drive needs to be optional here
            //setup: {
            //    type: TapeBackupJobSetup,
            //    flatten: true,
            //},

            store: {
                schema: DATASTORE_SCHEMA,
            },
            pool: {
                schema: MEDIA_POOL_NAME_SCHEMA,
            },
            drive: {
                schema: DRIVE_NAME_SCHEMA,
                optional: true,
            },
            "eject-media": {
                description: "Eject media upon job completion.",
                type: bool,
                optional: true,
            },
            "export-media-set": {
                description: "Export media set upon job completion.",
                type: bool,
                optional: true,
            },
            "latest-only": {
                description: "Backup latest snapshots only.",
                type: bool,
                optional: true,
            },
            "notify-user": {
                optional: true,
                type: Userid,
            },
            groups: {
                schema: GROUP_FILTER_LIST_SCHEMA,
                optional: true,
            },
            ns: {
                type: BackupNamespace,
                optional: true,
            },
            "max-depth": {
                schema: NS_MAX_DEPTH_SCHEMA,
                optional: true,
            },
            "force-media-set": {
                description: "Ignore the allocation policy and start a new media-set.",
                optional: true,
                type: bool,
                default: false,
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        },
    },
)]
/// Backup datastore to tape media pool
async fn backup(mut param: Value) -> Result<(), Error> {
    let output_format = extract_output_format(&mut param);

    let (config, _digest) = pbs_config::drive::config()?;

    param["drive"] = extract_drive_name(&mut param, &config)?.into();

    let client = connect_to_localhost()?;

    let result = client.post("api2/json/tape/backup", Some(param)).await?;

    view_task_result(&client, result, &output_format).await?;

    Ok(())
}

#[api(
   input: {
        properties: {
            store: {
                schema: DATASTORE_MAP_LIST_SCHEMA,
            },
            "namespaces": {
                description: "List of namespace to restore.",
                type: Array,
                optional: true,
                items: {
                    schema: TAPE_RESTORE_NAMESPACE_SCHEMA,
                },
            },
            drive: {
                schema: DRIVE_NAME_SCHEMA,
                optional: true,
            },
            "media-set": {
                description: "Media set UUID.",
                type: String,
            },
            "notify-user": {
                type: Userid,
                optional: true,
            },
            "snapshots": {
                description: "List of snapshots.",
                type: Array,
                optional: true,
                items: {
                    schema: TAPE_RESTORE_SNAPSHOT_SCHEMA,
                },
            },
            owner: {
                type: Authid,
                optional: true,
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        },
    },
)]
/// Restore data from media-set
async fn restore(mut param: Value) -> Result<(), Error> {
    let output_format = extract_output_format(&mut param);

    let (config, _digest) = pbs_config::drive::config()?;

    param["drive"] = extract_drive_name(&mut param, &config)?.into();

    let client = connect_to_localhost()?;

    let result = client.post("api2/json/tape/restore", Some(param)).await?;

    view_task_result(&client, result, &output_format).await?;

    Ok(())
}

#[api(
    input: {
        properties: {
            drive: {
                schema: DRIVE_NAME_SCHEMA,
                optional: true,
            },
            force: {
                description: "Force overriding existing index.",
                type: bool,
                optional: true,
            },
            scan: {
                description: "Re-read the whole tape to reconstruct the catalog instead of restoring saved versions.",
                type: bool,
                optional: true,
            },
            verbose: {
                description: "Verbose mode - log all found chunks.",
                type: bool,
                optional: true,
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        },
    },
)]
/// Scan media and record content
async fn catalog_media(mut param: Value) -> Result<(), Error> {
    let output_format = extract_output_format(&mut param);

    let (config, _digest) = pbs_config::drive::config()?;

    let drive = extract_drive_name(&mut param, &config)?;

    let client = connect_to_localhost()?;

    let path = format!("api2/json/tape/drive/{}/catalog", drive);
    let result = client.post(&path, Some(param)).await?;

    view_task_result(&client, result, &output_format).await?;

    Ok(())
}

fn main() {
    init_cli_logger("PBS_LOG", "info");

    let cmd_def = CliCommandMap::new()
        .insert(
            "backup",
            CliCommand::new(&API_METHOD_BACKUP)
                .arg_param(&["store", "pool"])
                .completion_cb("drive", complete_drive_name)
                .completion_cb("store", complete_datastore_name)
                .completion_cb("pool", complete_pool_name)
                .completion_cb("groups", complete_datastore_group_filter),
        )
        .insert(
            "restore",
            CliCommand::new(&API_METHOD_RESTORE)
                .arg_param(&["media-set", "store", "snapshots"])
                .completion_cb("store", complete_datastore_name)
                .completion_cb("media-set", complete_media_set_uuid)
                .completion_cb("snapshots", complete_media_set_snapshots),
        )
        .insert(
            "barcode-label",
            CliCommand::new(&API_METHOD_BARCODE_LABEL_MEDIA)
                .completion_cb("drive", complete_drive_name)
                .completion_cb("pool", complete_pool_name),
        )
        .insert(
            "rewind",
            CliCommand::new(&API_METHOD_REWIND).completion_cb("drive", complete_drive_name),
        )
        .insert(
            "scan",
            CliCommand::new(&API_METHOD_DEBUG_SCAN).completion_cb("drive", complete_drive_name),
        )
        .insert(
            "status",
            CliCommand::new(&API_METHOD_STATUS).completion_cb("drive", complete_drive_name),
        )
        .insert(
            "eod",
            CliCommand::new(&API_METHOD_MOVE_TO_EOM).completion_cb("drive", complete_drive_name),
        )
        .insert(
            "format",
            CliCommand::new(&API_METHOD_FORMAT_MEDIA).completion_cb("drive", complete_drive_name),
        )
        .insert(
            "eject",
            CliCommand::new(&API_METHOD_EJECT_MEDIA).completion_cb("drive", complete_drive_name),
        )
        .insert(
            "inventory",
            CliCommand::new(&API_METHOD_INVENTORY).completion_cb("drive", complete_drive_name),
        )
        .insert(
            "read-label",
            CliCommand::new(&API_METHOD_READ_LABEL).completion_cb("drive", complete_drive_name),
        )
        .insert(
            "catalog",
            CliCommand::new(&API_METHOD_CATALOG_MEDIA).completion_cb("drive", complete_drive_name),
        )
        .insert(
            "cartridge-memory",
            CliCommand::new(&API_METHOD_CARTRIDGE_MEMORY)
                .completion_cb("drive", complete_drive_name),
        )
        .insert(
            "volume-statistics",
            CliCommand::new(&API_METHOD_VOLUME_STATISTICS)
                .completion_cb("drive", complete_drive_name),
        )
        .insert(
            "clean",
            CliCommand::new(&API_METHOD_CLEAN_DRIVE).completion_cb("drive", complete_drive_name),
        )
        .insert(
            "label",
            CliCommand::new(&API_METHOD_LABEL_MEDIA)
                .completion_cb("drive", complete_drive_name)
                .completion_cb("pool", complete_pool_name),
        )
        .insert("changer", changer_commands())
        .insert("drive", drive_commands())
        .insert("pool", pool_commands())
        .insert("media", media_commands())
        .insert("key", encryption_key_commands())
        .insert("backup-job", backup_job_commands())
        .insert(
            "load-media",
            CliCommand::new(&API_METHOD_LOAD_MEDIA)
                .arg_param(&["label-text"])
                .completion_cb("drive", complete_drive_name)
                .completion_cb("label-text", complete_media_label_text),
        )
        .insert(
            "load-media-from-slot",
            CliCommand::new(&API_METHOD_LOAD_MEDIA_FROM_SLOT)
                .arg_param(&["source-slot"])
                .completion_cb("drive", complete_drive_name),
        )
        .insert(
            "unload",
            CliCommand::new(&API_METHOD_UNLOAD_MEDIA).completion_cb("drive", complete_drive_name),
        )
        .insert(
            "export-media",
            CliCommand::new(&API_METHOD_EXPORT_MEDIA)
                .arg_param(&["label-text"])
                .completion_cb("drive", complete_drive_name)
                .completion_cb("label-text", complete_media_label_text),
        );

    let mut rpcenv = CliEnvironment::new();
    rpcenv.set_auth_id(Some(String::from("root@pam")));

    proxmox_async::runtime::main(run_async_cli_command(cmd_def, rpcenv));
}
