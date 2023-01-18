use anyhow::Error;
use serde_json::{json, Value};

use proxmox_router::cli::*;
use proxmox_schema::api;

use pbs_api_types::percent_encoding::percent_encode_component;
use pbs_client::display_task_log;
use pbs_tools::json::required_string_param;

use pbs_api_types::UPID;

use crate::{complete_repository, connect, extract_repository_from_value, REPO_URL_SCHEMA};

#[api(
    input: {
        properties: {
            repository: {
                schema: REPO_URL_SCHEMA,
                optional: true,
            },
            limit: {
                description: "The maximal number of tasks to list.",
                type: Integer,
                optional: true,
                minimum: 1,
                maximum: 1000,
                default: 50,
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
            all: {
                type: Boolean,
                description: "Also list stopped tasks.",
                optional: true,
            },
        }
    }
)]
/// List running server tasks for this repo user
async fn task_list(param: Value) -> Result<Value, Error> {
    let output_format = get_output_format(&param);

    let repo = extract_repository_from_value(&param)?;
    let client = connect(&repo)?;

    let limit = param["limit"].as_u64().unwrap_or(50) as usize;
    let running = !param["all"].as_bool().unwrap_or(false);

    let args = json!({
        "running": running,
        "start": 0,
        "limit": limit,
        "userfilter": repo.auth_id(),
        "store": repo.store(),
    });

    let mut result = client
        .get("api2/json/nodes/localhost/tasks", Some(args))
        .await?;
    let mut data = result["data"].take();

    let return_type = &pbs_api_types::NODE_TASKS_LIST_TASKS_RETURN_TYPE;

    use pbs_tools::format::{render_epoch, render_task_status};
    let options = default_table_format_options()
        .column(
            ColumnConfig::new("starttime")
                .right_align(false)
                .renderer(render_epoch),
        )
        .column(
            ColumnConfig::new("endtime")
                .right_align(false)
                .renderer(render_epoch),
        )
        .column(ColumnConfig::new("upid"))
        .column(ColumnConfig::new("status").renderer(render_task_status));

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
            upid: {
                type: UPID,
            },
        }
    }
)]
/// Display the task log.
async fn task_log(param: Value) -> Result<Value, Error> {
    let repo = extract_repository_from_value(&param)?;
    let upid = required_string_param(&param, "upid")?;

    let client = connect(&repo)?;

    display_task_log(&client, upid, true, false).await?;

    Ok(Value::Null)
}

#[api(
    input: {
        properties: {
            repository: {
                schema: REPO_URL_SCHEMA,
                optional: true,
            },
            upid: {
                type: UPID,
            },
        }
    }
)]
/// Try to stop a specific task.
async fn task_stop(param: Value) -> Result<Value, Error> {
    let repo = extract_repository_from_value(&param)?;
    let upid_str = required_string_param(&param, "upid")?;

    let client = connect(&repo)?;

    let path = format!(
        "api2/json/nodes/localhost/tasks/{}",
        percent_encode_component(upid_str)
    );
    let _ = client.delete(&path, None).await?;

    Ok(Value::Null)
}

pub fn task_mgmt_cli() -> CliCommandMap {
    let task_list_cmd_def =
        CliCommand::new(&API_METHOD_TASK_LIST).completion_cb("repository", complete_repository);

    let task_log_cmd_def = CliCommand::new(&API_METHOD_TASK_LOG).arg_param(&["upid"]);

    let task_stop_cmd_def = CliCommand::new(&API_METHOD_TASK_STOP).arg_param(&["upid"]);

    CliCommandMap::new()
        .insert("log", task_log_cmd_def)
        .insert("list", task_list_cmd_def)
        .insert("stop", task_stop_cmd_def)
}
