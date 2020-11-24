use anyhow::Error;
use serde_json::{json, Value};

use proxmox::api::{api, cli::*};
use proxmox_backup::tools;

use crate::{
    complete_backup_snapshot, connect, extract_repository_from_value, BackupDir, REPO_URL_SCHEMA,
};

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
    CliCommandMap::new().insert("notes", notes_cli())
}
