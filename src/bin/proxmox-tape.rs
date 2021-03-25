use anyhow::{format_err, Error};
use serde_json::{json, Value};

use proxmox::{
    api::{
        api,
        cli::*,
        RpcEnvironment,
        section_config::SectionConfigData,
    },
    tools::{
        time::strftime_local,
        io::ReadExt,
    },
};

use proxmox_backup::{
    tools::format::{
        HumanByte,
        render_epoch,
        render_bytes_human_readable,
    },
    client::{
        connect_to_localhost,
        view_task_result,
    },
    api2::{
        self,
        types::{
            Authid,
            DATASTORE_SCHEMA,
            DATASTORE_MAP_LIST_SCHEMA,
            DRIVE_NAME_SCHEMA,
            MEDIA_LABEL_SCHEMA,
            MEDIA_POOL_NAME_SCHEMA,
            Userid,
        },
    },
    config::{
        self,
        datastore::complete_datastore_name,
        drive::complete_drive_name,
        media_pool::complete_pool_name,
    },
    tape::{
        drive::{
            open_drive,
            lock_tape_device,
            set_tape_device_state,
        },
        complete_media_label_text,
        complete_media_set_uuid,
        file_formats::{
            PROXMOX_BACKUP_CONTENT_HEADER_MAGIC_1_0,
            MediaContentHeader,
            proxmox_tape_magic_to_text,
        },
    },
};

mod proxmox_tape;
use proxmox_tape::*;

pub fn extract_drive_name(
    param: &mut Value,
    config: &SectionConfigData,
) -> Result<String, Error> {

    let drive = param["drive"]
        .as_str()
        .map(String::from)
        .or_else(|| std::env::var("PROXMOX_TAPE_DRIVE").ok())
        .or_else(||  {

            let mut drive_names = Vec::new();

            for (name, (section_type, _)) in config.sections.iter() {

                if !(section_type == "linux" || section_type == "virtual") { continue; }
                drive_names.push(name);
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
/// Erase media
async fn erase_media(mut param: Value) -> Result<(), Error> {

    let output_format = get_output_format(&param);

    let (config, _digest) = config::drive::config()?;

    let drive = extract_drive_name(&mut param, &config)?;

    let mut client = connect_to_localhost()?;

    let path = format!("api2/json/tape/drive/{}/erase-media", drive);
    let result = client.post(&path, Some(param)).await?;

    view_task_result(&mut client, result, &output_format).await?;

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

    let output_format = get_output_format(&param);

    let (config, _digest) = config::drive::config()?;

    let drive = extract_drive_name(&mut param, &config)?;

    let mut client = connect_to_localhost()?;

    let path = format!("api2/json/tape/drive/{}/rewind", drive);
    let result = client.post(&path, Some(param)).await?;

    view_task_result(&mut client, result, &output_format).await?;

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

    let output_format = get_output_format(&param);

    let (config, _digest) = config::drive::config()?;

    let drive = extract_drive_name(&mut param, &config)?;

    let mut client = connect_to_localhost()?;

    let path = format!("api2/json/tape/drive/{}/eject-media", drive);
    let result = client.post(&path, Some(param)).await?;

    view_task_result(&mut client, result, &output_format).await?;

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

    let output_format = get_output_format(&param);

    let (config, _digest) = config::drive::config()?;

    let drive = extract_drive_name(&mut param, &config)?;

    let mut client = connect_to_localhost()?;

    let path = format!("api2/json/tape/drive/{}/load-media", drive);
    let result = client.post(&path, Some(param)).await?;

    view_task_result(&mut client, result, &output_format).await?;

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

    let (config, _digest) = config::drive::config()?;

    let drive = extract_drive_name(&mut param, &config)?;

    let mut client = connect_to_localhost()?;

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

    let (config, _digest) = config::drive::config()?;

    let drive = extract_drive_name(&mut param, &config)?;

    let mut client = connect_to_localhost()?;

    let path = format!("api2/json/tape/drive/{}/load-slot", drive);
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

    let output_format = get_output_format(&param);

    let (config, _digest) = config::drive::config()?;

    let drive = extract_drive_name(&mut param, &config)?;

    let mut client = connect_to_localhost()?;

    let path = format!("api2/json/tape/drive/{}/unload", drive);
    let result = client.post(&path, Some(param)).await?;

    view_task_result(&mut client, result, &output_format).await?;

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

    let output_format = get_output_format(&param);

    let (config, _digest) = config::drive::config()?;

    let drive = extract_drive_name(&mut param, &config)?;

    let mut client = connect_to_localhost()?;

    let path = format!("api2/json/tape/drive/{}/label-media", drive);
    let result = client.post(&path, Some(param)).await?;

    view_task_result(&mut client, result, &output_format).await?;

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

    let output_format = get_output_format(&param);

    let (config, _digest) = config::drive::config()?;

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
        .column(ColumnConfig::new("encryption-key-fingerprint"))
        ;

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
            },
            "read-all-labels": {
                description: "Load all tapes and try read labels (even if already inventoried)",
                type: bool,
                optional: true,
            },
        },
    },
)]
/// List (and update) media labels (Changer Inventory)
async fn inventory(
    read_labels: Option<bool>,
    read_all_labels: Option<bool>,
    mut param: Value,
) -> Result<(), Error> {

    let output_format = get_output_format(&param);

    let (config, _digest) = config::drive::config()?;
    let drive = extract_drive_name(&mut param, &config)?;

    let do_read = read_labels.unwrap_or(false) || read_all_labels.unwrap_or(false);

    let mut client = connect_to_localhost()?;

    let path = format!("api2/json/tape/drive/{}/inventory", drive);

    if do_read {

        let mut param = json!({});
        if let Some(true) = read_all_labels {
            param["read-all-labels"] = true.into();
        }

        let result = client.put(&path, Some(param)).await?; // update inventory
        view_task_result(&mut client, result, &output_format).await?;
    }

    let mut result = client.get(&path, None).await?;
    let mut data = result["data"].take();

    let info = &api2::tape::drive::API_METHOD_INVENTORY;

    let options = default_table_format_options()
        .column(ColumnConfig::new("label-text"))
        .column(ColumnConfig::new("uuid"))
        ;

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

    let output_format = get_output_format(&param);

    let (config, _digest) = config::drive::config()?;

    let drive = extract_drive_name(&mut param, &config)?;

    let mut client = connect_to_localhost()?;

    let path = format!("api2/json/tape/drive/{}/barcode-label-media", drive);
    let result = client.post(&path, Some(param)).await?;

    view_task_result(&mut client, result, &output_format).await?;

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

    let (config, _digest) = config::drive::config()?;

    let drive = extract_drive_name(&mut param, &config)?;

    let _lock = lock_tape_device(&config, &drive)?;
    set_tape_device_state(&drive, "moving to eom")?;

    let mut drive = open_drive(&config, &drive)?;

    drive.move_to_eom()?;

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

    let (config, _digest) = config::drive::config()?;

    let drive = extract_drive_name(&mut param, &config)?;

    let _lock = lock_tape_device(&config, &drive)?;
    set_tape_device_state(&drive, "debug scan")?;

    let mut drive = open_drive(&config, &drive)?;

    println!("rewinding tape");
    drive.rewind()?;

    loop {
        let file_number = drive.current_file_number()?;

        match drive.read_next_file()? {
            None => {
                println!("EOD");
                continue;
            },
            Some(mut reader) => {
                println!("got file number {}", file_number);

                let header: Result<MediaContentHeader, _> = unsafe { reader.read_le_value() };
                match header {
                    Ok(header) => {
                        if header.magic != PROXMOX_BACKUP_CONTENT_HEADER_MAGIC_1_0 {
                            println!("got MediaContentHeader with wrong magic: {:?}", header.magic);
                        } else if let Some(name) = proxmox_tape_magic_to_text(&header.content_magic) {
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
                let bytes = reader.skip_to_end()?;
                println!("skipped {}", HumanByte::from(bytes));
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

    let output_format = get_output_format(&param);

    let (config, _digest) = config::drive::config()?;

    let drive = extract_drive_name(&mut param, &config)?;

    let client = connect_to_localhost()?;

    let path = format!("api2/json/tape/drive/{}/cartridge-memory", drive);
    let mut result = client.get(&path, Some(param)).await?;
    let mut data = result["data"].take();

    let info = &api2::tape::drive::API_METHOD_CARTRIDGE_MEMORY;

    let options = default_table_format_options()
        .column(ColumnConfig::new("id"))
        .column(ColumnConfig::new("name"))
        .column(ColumnConfig::new("value"))
        ;

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

    let output_format = get_output_format(&param);

    let (config, _digest) = config::drive::config()?;

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

    let output_format = get_output_format(&param);

    let (config, _digest) = config::drive::config()?;

    let drive = extract_drive_name(&mut param, &config)?;

    let client = connect_to_localhost()?;

    let path = format!("api2/json/tape/drive/{}/status", drive);
    let mut result = client.get(&path, Some(param)).await?;
    let mut data = result["data"].take();

    let info = &api2::tape::drive::API_METHOD_STATUS;

    let render_percentage = |value: &Value, _record: &Value| {
        match value.as_f64() {
            Some(wearout) => Ok(format!("{:.2}%", wearout*100.0)),
            None => Ok(String::from("ERROR")), // should never happen
        }
    };

    let options = default_table_format_options()
        .column(ColumnConfig::new("blocksize"))
        .column(ColumnConfig::new("density"))
        .column(ColumnConfig::new("status"))
        .column(ColumnConfig::new("options"))
        .column(ColumnConfig::new("alert-flags"))
        .column(ColumnConfig::new("file-number"))
        .column(ColumnConfig::new("block-number"))
        .column(ColumnConfig::new("manufactured").renderer(render_epoch))
        .column(ColumnConfig::new("bytes-written").renderer(render_bytes_human_readable))
        .column(ColumnConfig::new("bytes-read").renderer(render_bytes_human_readable))
        .column(ColumnConfig::new("medium-passes"))
        .column(ColumnConfig::new("medium-wearout").renderer(render_percentage))
        .column(ColumnConfig::new("volume-mounts"))
        ;

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

    let output_format = get_output_format(&param);

    let (config, _digest) = config::drive::config()?;

    let drive = extract_drive_name(&mut param, &config)?;

    let mut client = connect_to_localhost()?;

    let path = format!("api2/json/tape/drive/{}/clean", drive);
    let result = client.put(&path, Some(param)).await?;

    view_task_result(&mut client, result, &output_format).await?;

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
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        },
    },
)]
/// Backup datastore to tape media pool
async fn backup(mut param: Value) -> Result<(), Error> {

    let output_format = get_output_format(&param);

    let (config, _digest) = config::drive::config()?;

    param["drive"] = extract_drive_name(&mut param, &config)?.into();

    let mut client = connect_to_localhost()?;

    let result = client.post("api2/json/tape/backup", Some(param)).await?;

    view_task_result(&mut client, result, &output_format).await?;

    Ok(())
}

#[api(
   input: {
        properties: {
            store: {
                schema: DATASTORE_MAP_LIST_SCHEMA,
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

    let output_format = get_output_format(&param);

    let (config, _digest) = config::drive::config()?;

    param["drive"] = extract_drive_name(&mut param, &config)?.into();

    let mut client = connect_to_localhost()?;

    let result = client.post("api2/json/tape/restore", Some(param)).await?;

    view_task_result(&mut client, result, &output_format).await?;

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
async fn catalog_media(mut param: Value)  -> Result<(), Error> {

    let output_format = get_output_format(&param);

    let (config, _digest) = config::drive::config()?;

    let drive = extract_drive_name(&mut param, &config)?;

    let mut client = connect_to_localhost()?;

    let path = format!("api2/json/tape/drive/{}/catalog", drive);
    let result = client.post(&path, Some(param)).await?;

    view_task_result(&mut client, result, &output_format).await?;

    Ok(())
}

fn main() {

    let cmd_def = CliCommandMap::new()
        .insert(
            "backup",
            CliCommand::new(&API_METHOD_BACKUP)
                .arg_param(&["store", "pool"])
                .completion_cb("drive", complete_drive_name)
                .completion_cb("store", complete_datastore_name)
                .completion_cb("pool", complete_pool_name)
        )
        .insert(
            "restore",
            CliCommand::new(&API_METHOD_RESTORE)
                .arg_param(&["media-set", "store"])
                .completion_cb("store", complete_datastore_name)
                .completion_cb("media-set", complete_media_set_uuid)
        )
        .insert(
            "barcode-label",
            CliCommand::new(&API_METHOD_BARCODE_LABEL_MEDIA)
                .completion_cb("drive", complete_drive_name)
                .completion_cb("pool", complete_pool_name)
        )
        .insert(
            "rewind",
            CliCommand::new(&API_METHOD_REWIND)
                .completion_cb("drive", complete_drive_name)
        )
        .insert(
            "scan",
            CliCommand::new(&API_METHOD_DEBUG_SCAN)
                .completion_cb("drive", complete_drive_name)
        )
        .insert(
            "status",
            CliCommand::new(&API_METHOD_STATUS)
                .completion_cb("drive", complete_drive_name)
        )
        .insert(
            "eod",
            CliCommand::new(&API_METHOD_MOVE_TO_EOM)
                .completion_cb("drive", complete_drive_name)
        )
        .insert(
            "erase",
            CliCommand::new(&API_METHOD_ERASE_MEDIA)
                .completion_cb("drive", complete_drive_name)
        )
        .insert(
            "eject",
            CliCommand::new(&API_METHOD_EJECT_MEDIA)
                .completion_cb("drive", complete_drive_name)
        )
        .insert(
            "inventory",
            CliCommand::new(&API_METHOD_INVENTORY)
                .completion_cb("drive", complete_drive_name)
        )
        .insert(
            "read-label",
            CliCommand::new(&API_METHOD_READ_LABEL)
                .completion_cb("drive", complete_drive_name)
        )
        .insert(
            "catalog",
            CliCommand::new(&API_METHOD_CATALOG_MEDIA)
                .completion_cb("drive", complete_drive_name)
        )
        .insert(
            "cartridge-memory",
            CliCommand::new(&API_METHOD_CARTRIDGE_MEMORY)
                .completion_cb("drive", complete_drive_name)
        )
        .insert(
            "volume-statistics",
            CliCommand::new(&API_METHOD_VOLUME_STATISTICS)
                .completion_cb("drive", complete_drive_name)
        )
        .insert(
            "clean",
            CliCommand::new(&API_METHOD_CLEAN_DRIVE)
                .completion_cb("drive", complete_drive_name)
        )
        .insert(
            "label",
            CliCommand::new(&API_METHOD_LABEL_MEDIA)
                .completion_cb("drive", complete_drive_name)
                .completion_cb("pool", complete_pool_name)

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
                .completion_cb("label-text", complete_media_label_text)
        )
        .insert(
            "load-media-from-slot",
            CliCommand::new(&API_METHOD_LOAD_MEDIA_FROM_SLOT)
                .arg_param(&["source-slot"])
                .completion_cb("drive", complete_drive_name)
        )
        .insert(
            "unload",
            CliCommand::new(&API_METHOD_UNLOAD_MEDIA)
                .completion_cb("drive", complete_drive_name)
        )
        .insert(
            "export-media",
            CliCommand::new(&API_METHOD_EXPORT_MEDIA)
                .arg_param(&["label-text"])
                .completion_cb("drive", complete_drive_name)
                .completion_cb("label-text", complete_media_label_text)
        )
        ;

    let mut rpcenv = CliEnvironment::new();
    rpcenv.set_auth_id(Some(String::from("root@pam")));

    proxmox_backup::tools::runtime::main(run_async_cli_command(cmd_def, rpcenv));
}
