use anyhow::Error;
use serde_json::Value;

use proxmox::api::{api, cli::*, RpcEnvironment, ApiHandler};

use proxmox_backup::config;
use proxmox_backup::api2::{self, types::* };

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

    let options = default_table_format_options();
    format_and_print_result_full(&mut data, &info.returns, &output_format, &options);

    Ok(Value::Null)
}

pub fn sync_job_commands() -> CommandLineInterface {

    let cmd_def = CliCommandMap::new()
        .insert("list", CliCommand::new(&API_METHOD_LIST_SYNC_JOBS))
        .insert("show",
                CliCommand::new(&API_METHOD_SHOW_SYNC_JOB)
                .arg_param(&["id"])
                .completion_cb("id", config::sync::complete_sync_job_id)
        )
        .insert("create",
                CliCommand::new(&api2::config::sync::API_METHOD_CREATE_SYNC_JOB)
                .arg_param(&["id"])
                .completion_cb("id", config::sync::complete_sync_job_id)
                .completion_cb("schedule", config::datastore::complete_calendar_event)
                .completion_cb("store", config::datastore::complete_datastore_name)
                .completion_cb("remote", pbs_config::remote::complete_remote_name)
                .completion_cb("remote-store", crate::complete_remote_datastore_name)
        )
        .insert("update",
                CliCommand::new(&api2::config::sync::API_METHOD_UPDATE_SYNC_JOB)
                .arg_param(&["id"])
                .completion_cb("id", config::sync::complete_sync_job_id)
                .completion_cb("schedule", config::datastore::complete_calendar_event)
                .completion_cb("store", config::datastore::complete_datastore_name)
                .completion_cb("remote-store", crate::complete_remote_datastore_name)
        )
        .insert("remove",
                CliCommand::new(&api2::config::sync::API_METHOD_DELETE_SYNC_JOB)
                .arg_param(&["id"])
                .completion_cb("id", config::sync::complete_sync_job_id)
        );

    cmd_def.into()
}
