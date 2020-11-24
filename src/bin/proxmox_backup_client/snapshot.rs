use anyhow::Error;
use serde_json::{json, Value};

use proxmox::api::{api, cli::*};
use proxmox_backup::{
    tools,
    api2::types::*,
    backup::{
        BackupGroup,
    }
};

use crate::{
    REPO_URL_SCHEMA,
    BackupDir,
    api_datastore_list_snapshots,
    complete_backup_snapshot,
    complete_backup_group,
    complete_repository,
    connect,
    extract_repository_from_value,
    record_repository,
};

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

    let info = &proxmox_backup::api2::admin::datastore::API_RETURN_SCHEMA_LIST_SNAPSHOTS;

    format_and_print_result_full(&mut data, info, &output_format, &options);

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
            "list", CliCommand::new(&API_METHOD_LIST_SNAPSHOTS)
                .arg_param(&["group"])
                .completion_cb("group", complete_backup_group)
                .completion_cb("repository", complete_repository)
        )
}
