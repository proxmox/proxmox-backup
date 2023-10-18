use std::sync::Arc;

use anyhow::Error;
use serde_json::{json, Value};

use proxmox_router::cli::*;
use proxmox_schema::api;
use proxmox_sys::fs::file_get_contents;

use pbs_api_types::{BackupGroup, BackupNamespace, CryptMode, SnapshotListItem};
use pbs_client::tools::key_source::get_encryption_key_password;
use pbs_datastore::DataBlob;
use pbs_key_config::decrypt_key;
use pbs_tools::crypt_config::CryptConfig;
use pbs_tools::json::required_string_param;

use crate::{
    api_datastore_list_snapshots, complete_backup_group, complete_backup_snapshot,
    complete_namespace, complete_repository, connect, crypto_parameters,
    extract_repository_from_value, optional_ns_param, record_repository, BackupDir, KEYFD_SCHEMA,
    KEYFILE_SCHEMA, REPO_URL_SCHEMA,
};

fn snapshot_args(ns: &BackupNamespace, snapshot: &BackupDir) -> Result<Value, Error> {
    let mut args = serde_json::to_value(snapshot)?;
    if !ns.is_root() {
        args["ns"] = serde_json::to_value(ns)?;
    }
    Ok(args)
}

#[api(
   input: {
        properties: {
            repository: {
                schema: REPO_URL_SCHEMA,
                optional: true,
            },
            ns: {
                type: BackupNamespace,
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

    let group: Option<BackupGroup> = param["group"]
        .as_str()
        .map(|group| group.parse())
        .transpose()?;

    let backup_ns = optional_ns_param(&param)?;

    let mut data =
        api_datastore_list_snapshots(&client, repo.store(), &backup_ns, group.as_ref()).await?;

    record_repository(&repo);

    let render_snapshot_path = |_v: &Value, record: &Value| -> Result<String, Error> {
        let item: SnapshotListItem = serde_json::from_value(record.to_owned())?;
        Ok(item.backup.to_string())
    };

    let render_files = |_v: &Value, record: &Value| -> Result<String, Error> {
        let item: SnapshotListItem = serde_json::from_value(record.to_owned())?;
        let mut filenames = Vec::new();
        for file in &item.files {
            filenames.push(file.filename.to_string());
        }
        Ok(pbs_tools::format::render_backup_file_list(&filenames[..]))
    };

    let options = default_table_format_options()
        .sortby("backup-type", false)
        .sortby("backup-id", false)
        .sortby("backup-time", false)
        .column(
            ColumnConfig::new("backup-id")
                .renderer(render_snapshot_path)
                .header("snapshot"),
        )
        .column(ColumnConfig::new("size").renderer(pbs_tools::format::render_bytes_human_readable))
        .column(ColumnConfig::new("files").renderer(render_files));

    let return_type = &pbs_api_types::ADMIN_DATASTORE_LIST_SNAPSHOTS_RETURN_TYPE;

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
            ns: {
                type: BackupNamespace,
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

    let backup_ns = optional_ns_param(&param)?;
    let path = required_string_param(&param, "snapshot")?;
    let snapshot: BackupDir = path.parse()?;

    let output_format = get_output_format(&param);

    let client = connect(&repo)?;

    let path = format!("api2/json/admin/datastore/{}/files", repo.store());

    let mut result = client
        .get(&path, Some(snapshot_args(&backup_ns, &snapshot)?))
        .await?;

    record_repository(&repo);

    let return_type = &pbs_api_types::ADMIN_DATASTORE_LIST_SNAPSHOT_FILES_RETURN_TYPE;

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
            ns: {
                type: BackupNamespace,
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
async fn forget_snapshots(param: Value) -> Result<(), Error> {
    let repo = extract_repository_from_value(&param)?;

    let backup_ns = optional_ns_param(&param)?;
    let path = required_string_param(&param, "snapshot")?;
    let snapshot: BackupDir = path.parse()?;

    let client = connect(&repo)?;

    let path = format!("api2/json/admin/datastore/{}/snapshots", repo.store());

    client
        .delete(&path, Some(snapshot_args(&backup_ns, &snapshot)?))
        .await?;

    record_repository(&repo);

    Ok(())
}

#[api(
    input: {
        properties: {
            repository: {
                schema: REPO_URL_SCHEMA,
                optional: true,
            },
            ns: {
                type: BackupNamespace,
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
    let logfile = required_string_param(&param, "logfile")?;
    let repo = extract_repository_from_value(&param)?;

    let backup_ns = optional_ns_param(&param)?;
    let snapshot = required_string_param(&param, "snapshot")?;
    let snapshot: BackupDir = snapshot.parse()?;

    let client = connect(&repo)?;

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
        CryptMode::Encrypt => {
            DataBlob::encode(&data, crypt_config.as_ref().map(Arc::as_ref), true)?
        }
    };

    let raw_data = blob.into_inner();

    let path = format!(
        "api2/json/admin/datastore/{}/upload-backup-log",
        repo.store()
    );

    let args = snapshot_args(&backup_ns, &snapshot)?;
    let body = hyper::Body::from(raw_data);

    client
        .upload("application/octet-stream", body, &path, Some(args))
        .await
}

#[api(
    input: {
        properties: {
            repository: {
                schema: REPO_URL_SCHEMA,
                optional: true,
            },
            ns: {
                type: BackupNamespace,
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
    let path = required_string_param(&param, "snapshot")?;

    let backup_ns = optional_ns_param(&param)?;
    let snapshot: BackupDir = path.parse()?;
    let client = connect(&repo)?;

    let path = format!("api2/json/admin/datastore/{}/notes", repo.store());

    let args = snapshot_args(&backup_ns, &snapshot)?;

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
            ns: {
                type: BackupNamespace,
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
    let path = required_string_param(&param, "snapshot")?;
    let notes = required_string_param(&param, "notes")?;

    let backup_ns = optional_ns_param(&param)?;
    let snapshot: BackupDir = path.parse()?;
    let client = connect(&repo)?;

    let path = format!("api2/json/admin/datastore/{}/notes", repo.store());

    let mut args = snapshot_args(&backup_ns, &snapshot)?;
    args["notes"] = Value::from(notes);

    client.put(&path, Some(args)).await?;

    Ok(Value::Null)
}

#[api(
    input: {
        properties: {
            repository: {
                schema: REPO_URL_SCHEMA,
                optional: true,
            },
            ns: {
                type: BackupNamespace,
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
/// Show protection status of the specified snapshot
async fn show_protection(param: Value) -> Result<(), Error> {
    let repo = extract_repository_from_value(&param)?;
    let path = required_string_param(&param, "snapshot")?;

    let backup_ns = optional_ns_param(&param)?;
    let snapshot: BackupDir = path.parse()?;
    let client = connect(&repo)?;

    let path = format!("api2/json/admin/datastore/{}/protected", repo.store());

    let args = snapshot_args(&backup_ns, &snapshot)?;

    let output_format = get_output_format(&param);

    let mut result = client.get(&path, Some(args)).await?;

    let protected = result["data"].take();

    if output_format == "text" {
        if let Some(protected) = protected.as_bool() {
            println!("{}", protected);
        }
    } else {
        format_and_print_result(
            &json!({
                "protected": protected,
            }),
            &output_format,
        );
    }

    Ok(())
}

#[api(
    input: {
        properties: {
            repository: {
                schema: REPO_URL_SCHEMA,
                optional: true,
            },
            ns: {
                type: BackupNamespace,
                optional: true,
            },
            snapshot: {
                type: String,
                description: "Snapshot path.",
            },
            protected: {
                type: bool,
                description: "The protection status.",
            },
        }
    }
)]
/// Update Protection Status of a snapshot
async fn update_protection(protected: bool, param: Value) -> Result<(), Error> {
    let repo = extract_repository_from_value(&param)?;
    let path = required_string_param(&param, "snapshot")?;

    let backup_ns = optional_ns_param(&param)?;
    let snapshot: BackupDir = path.parse()?;
    let client = connect(&repo)?;

    let path = format!("api2/json/admin/datastore/{}/protected", repo.store());

    let mut args = snapshot_args(&backup_ns, &snapshot)?;
    args["protected"] = Value::from(protected);

    client.put(&path, Some(args)).await?;

    Ok(())
}

fn protected_cli() -> CliCommandMap {
    CliCommandMap::new()
        .insert(
            "show",
            CliCommand::new(&API_METHOD_SHOW_PROTECTION)
                .arg_param(&["snapshot"])
                .completion_cb("ns", complete_namespace)
                .completion_cb("snapshot", complete_backup_snapshot),
        )
        .insert(
            "update",
            CliCommand::new(&API_METHOD_UPDATE_PROTECTION)
                .arg_param(&["snapshot", "protected"])
                .completion_cb("ns", complete_namespace)
                .completion_cb("snapshot", complete_backup_snapshot),
        )
}

fn notes_cli() -> CliCommandMap {
    CliCommandMap::new()
        .insert(
            "show",
            CliCommand::new(&API_METHOD_SHOW_NOTES)
                .arg_param(&["snapshot"])
                .completion_cb("ns", complete_namespace)
                .completion_cb("snapshot", complete_backup_snapshot),
        )
        .insert(
            "update",
            CliCommand::new(&API_METHOD_UPDATE_NOTES)
                .arg_param(&["snapshot", "notes"])
                .completion_cb("ns", complete_namespace)
                .completion_cb("snapshot", complete_backup_snapshot),
        )
}

pub fn snapshot_mgtm_cli() -> CliCommandMap {
    CliCommandMap::new()
        .insert("notes", notes_cli())
        .insert("protected", protected_cli())
        .insert(
            "list",
            CliCommand::new(&API_METHOD_LIST_SNAPSHOTS)
                .arg_param(&["group"])
                .completion_cb("ns", complete_namespace)
                .completion_cb("group", complete_backup_group)
                .completion_cb("repository", complete_repository),
        )
        .insert(
            "files",
            CliCommand::new(&API_METHOD_LIST_SNAPSHOT_FILES)
                .arg_param(&["snapshot"])
                .completion_cb("ns", complete_namespace)
                .completion_cb("repository", complete_repository)
                .completion_cb("snapshot", complete_backup_snapshot),
        )
        .insert(
            "forget",
            CliCommand::new(&API_METHOD_FORGET_SNAPSHOTS)
                .arg_param(&["snapshot"])
                .completion_cb("ns", complete_namespace)
                .completion_cb("repository", complete_repository)
                .completion_cb("snapshot", complete_backup_snapshot),
        )
        .insert(
            "upload-log",
            CliCommand::new(&API_METHOD_UPLOAD_LOG)
                .arg_param(&["snapshot", "logfile"])
                .completion_cb("ns", complete_namespace)
                .completion_cb("snapshot", complete_backup_snapshot)
                .completion_cb("logfile", complete_file_name)
                .completion_cb("keyfile", complete_file_name)
                .completion_cb("repository", complete_repository),
        )
}
