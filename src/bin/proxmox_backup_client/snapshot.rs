use std::sync::Arc;

use anyhow::Error;
use serde_json::{json, Value};

use proxmox::{
    api::{api, cli::*},
    tools::fs::file_get_contents,
};

use proxmox_backup::{
    tools,
    api2::types::*,
    backup::{
        CryptMode,
        CryptConfig,
        DataBlob,
        BackupGroup,
        decrypt_key,
    }
};

use crate::{
    REPO_URL_SCHEMA,
    KEYFILE_SCHEMA,
    KEYFD_SCHEMA,
    BackupDir,
    api_datastore_list_snapshots,
    complete_backup_snapshot,
    complete_backup_group,
    complete_repository,
    connect,
    crypto_parameters,
    extract_repository_from_value,
    record_repository,
};

use crate::proxmox_client_tools::key_source::get_encryption_key_password;

#[api(
   input: {
        properties: {
            repository: {
                schema: REPO_URL_SCHEMA,
                optional: true,
            },
            group: {
                type: String,
                description: "Backup group.",
                optional: true,
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        }
   }
)]
/// List backup snapshots.
async fn list_snapshots(param: Value) -> Result<Value, Error> {

    let repo = extract_repository_from_value(&param)?;

    let output_format = get_output_format(&param);

    let client = connect(&repo)?;

    let group: Option<BackupGroup> = if let Some(path) = param["group"].as_str() {
        Some(path.parse()?)
    } else {
        None
    };

    let mut data = api_datastore_list_snapshots(&client, repo.store(), group).await?;

    record_repository(&repo);

    let render_snapshot_path = |_v: &Value, record: &Value| -> Result<String, Error> {
        let item: SnapshotListItem = serde_json::from_value(record.to_owned())?;
        let snapshot = BackupDir::new(item.backup_type, item.backup_id, item.backup_time)?;
        Ok(snapshot.relative_path().to_str().unwrap().to_owned())
    };

    let render_files = |_v: &Value, record: &Value| -> Result<String, Error> {
        let item: SnapshotListItem = serde_json::from_value(record.to_owned())?;
        let mut filenames = Vec::new();
        for file in &item.files {
            filenames.push(file.filename.to_string());
        }
        Ok(tools::format::render_backup_file_list(&filenames[..]))
    };

    let options = default_table_format_options()
        .sortby("backup-type", false)
        .sortby("backup-id", false)
        .sortby("backup-time", false)
        .column(ColumnConfig::new("backup-id").renderer(render_snapshot_path).header("snapshot"))
        .column(ColumnConfig::new("size").renderer(tools::format::render_bytes_human_readable))
        .column(ColumnConfig::new("files").renderer(render_files))
        ;

    let return_type = &proxmox_backup::api2::admin::datastore::API_METHOD_LIST_SNAPSHOTS.returns;

    format_and_print_result_full(&mut data, return_type, &output_format, &options);

    Ok(Value::Null)
}

#[api(
   input: {
        properties: {
            repository: {
                schema: REPO_URL_SCHEMA,
                optional: true,
            },
            snapshot: {
                type: String,
                description: "Snapshot path.",
             },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        }
   }
)]
/// List snapshot files.
async fn list_snapshot_files(param: Value) -> Result<Value, Error> {

    let repo = extract_repository_from_value(&param)?;

    let path = tools::required_string_param(&param, "snapshot")?;
    let snapshot: BackupDir = path.parse()?;

    let output_format = get_output_format(&param);

    let client = connect(&repo)?;

    let path = format!("api2/json/admin/datastore/{}/files", repo.store());

    let mut result = client.get(&path, Some(json!({
        "backup-type": snapshot.group().backup_type(),
        "backup-id": snapshot.group().backup_id(),
        "backup-time": snapshot.backup_time(),
    }))).await?;

    record_repository(&repo);

    let return_type =
        &proxmox_backup::api2::admin::datastore::API_METHOD_LIST_SNAPSHOT_FILES.returns;

    let mut data: Value = result["data"].take();

    let options = default_table_format_options();

    format_and_print_result_full(&mut data, return_type, &output_format, &options);

    Ok(Value::Null)
}

#[api(
   input: {
        properties: {
            repository: {
                schema: REPO_URL_SCHEMA,
                optional: true,
            },
            snapshot: {
                type: String,
                description: "Snapshot path.",
             },
        }
   }
)]
/// Forget (remove) backup snapshots.
async fn forget_snapshots(param: Value) -> Result<Value, Error> {

    let repo = extract_repository_from_value(&param)?;

    let path = tools::required_string_param(&param, "snapshot")?;
    let snapshot: BackupDir = path.parse()?;

    let mut client = connect(&repo)?;

    let path = format!("api2/json/admin/datastore/{}/snapshots", repo.store());

    let result = client.delete(&path, Some(json!({
        "backup-type": snapshot.group().backup_type(),
        "backup-id": snapshot.group().backup_id(),
        "backup-time": snapshot.backup_time(),
    }))).await?;

    record_repository(&repo);

    Ok(result)
}

#[api(
   input: {
       properties: {
           repository: {
               schema: REPO_URL_SCHEMA,
               optional: true,
           },
           snapshot: {
               type: String,
               description: "Group/Snapshot path.",
           },
           logfile: {
               type: String,
               description: "The path to the log file you want to upload.",
           },
           keyfile: {
               schema: KEYFILE_SCHEMA,
               optional: true,
           },
           "keyfd": {
               schema: KEYFD_SCHEMA,
               optional: true,
           },
           "crypt-mode": {
               type: CryptMode,
               optional: true,
           },
       }
   }
)]
/// Upload backup log file.
async fn upload_log(param: Value) -> Result<Value, Error> {

    let logfile = tools::required_string_param(&param, "logfile")?;
    let repo = extract_repository_from_value(&param)?;

    let snapshot = tools::required_string_param(&param, "snapshot")?;
    let snapshot: BackupDir = snapshot.parse()?;

    let mut client = connect(&repo)?;

    let crypto = crypto_parameters(&param)?;

    let crypt_config = match crypto.enc_key {
        None => None,
        Some(key) => {
            let (key, _created, _) = decrypt_key(&key.key, &get_encryption_key_password)?;
            let crypt_config = CryptConfig::new(key)?;
            Some(Arc::new(crypt_config))
        }
    };

    let data = file_get_contents(logfile)?;

    // fixme: howto sign log?
    let blob = match crypto.mode {
        CryptMode::None | CryptMode::SignOnly => DataBlob::encode(&data, None, true)?,
        CryptMode::Encrypt => DataBlob::encode(&data, crypt_config.as_ref().map(Arc::as_ref), true)?,
    };

    let raw_data = blob.into_inner();

    let path = format!("api2/json/admin/datastore/{}/upload-backup-log", repo.store());

    let args = json!({
        "backup-type": snapshot.group().backup_type(),
        "backup-id":  snapshot.group().backup_id(),
        "backup-time": snapshot.backup_time(),
    });

    let body = hyper::Body::from(raw_data);

    client.upload("application/octet-stream", body, &path, Some(args)).await
}

#[api(
    input: {
        properties: {
            repository: {
                schema: REPO_URL_SCHEMA,
                optional: true,
            },
            snapshot: {
                type: String,
                description: "Snapshot path.",
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        }
    }
)]
/// Show notes
async fn show_notes(param: Value) -> Result<Value, Error> {
    let repo = extract_repository_from_value(&param)?;
    let path = tools::required_string_param(&param, "snapshot")?;

    let snapshot: BackupDir = path.parse()?;
    let client = connect(&repo)?;

    let path = format!("api2/json/admin/datastore/{}/notes", repo.store());

    let args = json!({
        "backup-type": snapshot.group().backup_type(),
        "backup-id": snapshot.group().backup_id(),
        "backup-time": snapshot.backup_time(),
    });

    let output_format = get_output_format(&param);

    let mut result = client.get(&path, Some(args)).await?;

    let notes = result["data"].take();

    if output_format == "text" {
        if let Some(notes) = notes.as_str() {
            println!("{}", notes);
        }
    } else {
        format_and_print_result(
            &json!({
                "notes": notes,
            }),
            &output_format,
        );
    }

    Ok(Value::Null)
}

#[api(
    input: {
        properties: {
            repository: {
                schema: REPO_URL_SCHEMA,
                optional: true,
            },
            snapshot: {
                type: String,
                description: "Snapshot path.",
            },
            notes: {
                type: String,
                description: "The Notes.",
            },
        }
    }
)]
/// Update Notes
async fn update_notes(param: Value) -> Result<Value, Error> {
    let repo = extract_repository_from_value(&param)?;
    let path = tools::required_string_param(&param, "snapshot")?;
    let notes = tools::required_string_param(&param, "notes")?;

    let snapshot: BackupDir = path.parse()?;
    let mut client = connect(&repo)?;

    let path = format!("api2/json/admin/datastore/{}/notes", repo.store());

    let args = json!({
        "backup-type": snapshot.group().backup_type(),
        "backup-id": snapshot.group().backup_id(),
        "backup-time": snapshot.backup_time(),
        "notes": notes,
    });

    client.put(&path, Some(args)).await?;

    Ok(Value::Null)
}

fn notes_cli() -> CliCommandMap {
    CliCommandMap::new()
        .insert(
            "show",
            CliCommand::new(&API_METHOD_SHOW_NOTES)
                .arg_param(&["snapshot"])
                .completion_cb("snapshot", complete_backup_snapshot),
        )
        .insert(
            "update",
            CliCommand::new(&API_METHOD_UPDATE_NOTES)
                .arg_param(&["snapshot", "notes"])
                .completion_cb("snapshot", complete_backup_snapshot),
        )
}

pub fn snapshot_mgtm_cli() -> CliCommandMap {
    CliCommandMap::new()
        .insert("notes", notes_cli())
        .insert(
            "list",
            CliCommand::new(&API_METHOD_LIST_SNAPSHOTS)
                .arg_param(&["group"])
                .completion_cb("group", complete_backup_group)
                .completion_cb("repository", complete_repository)
        )
        .insert(
            "files",
            CliCommand::new(&API_METHOD_LIST_SNAPSHOT_FILES)
                .arg_param(&["snapshot"])
                .completion_cb("repository", complete_repository)
                .completion_cb("snapshot", complete_backup_snapshot)
        )
        .insert(
            "forget",
            CliCommand::new(&API_METHOD_FORGET_SNAPSHOTS)
                .arg_param(&["snapshot"])
                .completion_cb("repository", complete_repository)
                .completion_cb("snapshot", complete_backup_snapshot)
        )
        .insert(
            "upload-log",
            CliCommand::new(&API_METHOD_UPLOAD_LOG)
                .arg_param(&["snapshot", "logfile"])
                .completion_cb("snapshot", complete_backup_snapshot)
                .completion_cb("logfile", tools::complete_file_name)
                .completion_cb("keyfile", tools::complete_file_name)
                .completion_cb("repository", complete_repository)
        )
}
