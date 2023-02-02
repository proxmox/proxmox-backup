use anyhow::Error;
use serde_json::Value;

use proxmox_router::{cli::*, ApiHandler, RpcEnvironment};
use proxmox_schema::api;

use pbs_api_types::JOB_ID_SCHEMA;

use proxmox_backup::api2;

fn render_group_filter(value: &Value, _record: &Value) -> Result<String, Error> {
    if let Some(group_filters) = value.as_array() {
        let group_filters: Vec<&str> = group_filters.iter().filter_map(Value::as_str).collect();
        Ok(group_filters.join(" OR "))
    } else {
        Ok(String::from("all"))
    }
}

#[api(
    input: {
        properties: {
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        }
    }
)]
/// Sync job list.
fn list_sync_jobs(param: Value, rpcenv: &mut dyn RpcEnvironment) -> Result<Value, Error> {
    let output_format = get_output_format(&param);

    let info = &api2::config::sync::API_METHOD_LIST_SYNC_JOBS;
    let mut data = match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    let options = default_table_format_options()
        .column(ColumnConfig::new("id"))
        .column(ColumnConfig::new("store"))
        .column(ColumnConfig::new("remote"))
        .column(ColumnConfig::new("remote-store"))
        .column(ColumnConfig::new("schedule"))
        .column(ColumnConfig::new("group-filter").renderer(render_group_filter))
        .column(ColumnConfig::new("rate-in"))
        .column(ColumnConfig::new("comment"));

    format_and_print_result_full(&mut data, &info.returns, &output_format, &options);

    Ok(Value::Null)
}

#[api(
    input: {
        properties: {
            id: {
                schema: JOB_ID_SCHEMA,
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        }
    }
)]
/// Show sync job configuration
fn show_sync_job(param: Value, rpcenv: &mut dyn RpcEnvironment) -> Result<Value, Error> {
    let output_format = get_output_format(&param);

    let info = &api2::config::sync::API_METHOD_READ_SYNC_JOB;
    let mut data = match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    if let Some(groups) = data.get_mut("groups") {
        if let Ok(rendered) = render_group_filter(groups, groups) {
            *groups = Value::String(rendered);
        }
    }

    let options = default_table_format_options();
    format_and_print_result_full(&mut data, &info.returns, &output_format, &options);

    Ok(Value::Null)
}

#[api(
    input: {
        properties: {
            id: {
                schema: JOB_ID_SCHEMA,
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        }
    }
)]
/// Run the specified sync job
async fn run_sync_job(param: Value) -> Result<Value, Error> {
    crate::run_job("sync", param).await
}

pub fn sync_job_commands() -> CommandLineInterface {
    let cmd_def = CliCommandMap::new()
        .insert("list", CliCommand::new(&API_METHOD_LIST_SYNC_JOBS))
        .insert(
            "show",
            CliCommand::new(&API_METHOD_SHOW_SYNC_JOB)
                .arg_param(&["id"])
                .completion_cb("id", pbs_config::sync::complete_sync_job_id),
        )
        .insert(
            "create",
            CliCommand::new(&api2::config::sync::API_METHOD_CREATE_SYNC_JOB)
                .arg_param(&["id"])
                .completion_cb("id", pbs_config::sync::complete_sync_job_id)
                .completion_cb("schedule", pbs_config::datastore::complete_calendar_event)
                .completion_cb("store", pbs_config::datastore::complete_datastore_name)
                .completion_cb("ns", crate::complete_sync_local_datastore_namespace)
                .completion_cb("remote", pbs_config::remote::complete_remote_name)
                .completion_cb("remote-store", crate::complete_remote_datastore_name)
                .completion_cb(
                    "group-filter",
                    crate::complete_remote_datastore_group_filter,
                )
                .completion_cb("remote-ns", crate::complete_remote_datastore_namespace),
        )
        .insert(
            "update",
            CliCommand::new(&api2::config::sync::API_METHOD_UPDATE_SYNC_JOB)
                .arg_param(&["id"])
                .completion_cb("id", pbs_config::sync::complete_sync_job_id)
                .completion_cb("schedule", pbs_config::datastore::complete_calendar_event)
                .completion_cb("store", pbs_config::datastore::complete_datastore_name)
                .completion_cb("ns", crate::complete_sync_local_datastore_namespace)
                .completion_cb("remote-store", crate::complete_remote_datastore_name)
                .completion_cb(
                    "group-filter",
                    crate::complete_remote_datastore_group_filter,
                )
                .completion_cb("remote-ns", crate::complete_remote_datastore_namespace),
        )
        .insert(
            "run",
            CliCommand::new(&API_METHOD_RUN_SYNC_JOB)
                .arg_param(&["id"])
                .completion_cb("id", pbs_config::sync::complete_sync_job_id),
        )
        .insert(
            "remove",
            CliCommand::new(&api2::config::sync::API_METHOD_DELETE_SYNC_JOB)
                .arg_param(&["id"])
                .completion_cb("id", pbs_config::sync::complete_sync_job_id),
        );

    cmd_def.into()
}
