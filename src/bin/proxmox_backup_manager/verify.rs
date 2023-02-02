use anyhow::Error;
use serde_json::Value;

use proxmox_router::{cli::*, ApiHandler, RpcEnvironment};
use proxmox_schema::api;

use pbs_api_types::JOB_ID_SCHEMA;

use proxmox_backup::api2;

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
/// List all verification jobs
fn list_verification_jobs(param: Value, rpcenv: &mut dyn RpcEnvironment) -> Result<Value, Error> {
    let output_format = get_output_format(&param);

    let info = &api2::config::verify::API_METHOD_LIST_VERIFICATION_JOBS;
    let mut data = match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    let options = default_table_format_options()
        .column(ColumnConfig::new("id"))
        .column(ColumnConfig::new("store"))
        .column(ColumnConfig::new("schedule"))
        .column(ColumnConfig::new("ignore-verified"))
        .column(ColumnConfig::new("outdated-after"))
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
/// Show verification job configuration
fn show_verification_job(param: Value, rpcenv: &mut dyn RpcEnvironment) -> Result<Value, Error> {
    let output_format = get_output_format(&param);

    let info = &api2::config::verify::API_METHOD_READ_VERIFICATION_JOB;
    let mut data = match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

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
/// Run the specified verification job
async fn run_verification_job(param: Value) -> Result<Value, Error> {
    crate::run_job("verify", param).await
}

pub fn verify_job_commands() -> CommandLineInterface {
    let cmd_def = CliCommandMap::new()
        .insert("list", CliCommand::new(&API_METHOD_LIST_VERIFICATION_JOBS))
        .insert(
            "show",
            CliCommand::new(&API_METHOD_SHOW_VERIFICATION_JOB)
                .arg_param(&["id"])
                .completion_cb("id", pbs_config::verify::complete_verification_job_id),
        )
        .insert(
            "create",
            CliCommand::new(&api2::config::verify::API_METHOD_CREATE_VERIFICATION_JOB)
                .arg_param(&["id"])
                .completion_cb("id", pbs_config::verify::complete_verification_job_id)
                .completion_cb("schedule", pbs_config::datastore::complete_calendar_event)
                .completion_cb("store", pbs_config::datastore::complete_datastore_name),
        )
        .insert(
            "update",
            CliCommand::new(&api2::config::verify::API_METHOD_UPDATE_VERIFICATION_JOB)
                .arg_param(&["id"])
                .completion_cb("id", pbs_config::verify::complete_verification_job_id)
                .completion_cb("schedule", pbs_config::datastore::complete_calendar_event)
                .completion_cb("store", pbs_config::datastore::complete_datastore_name)
                .completion_cb("remote-store", crate::complete_remote_datastore_name),
        )
        .insert(
            "run",
            CliCommand::new(&API_METHOD_RUN_VERIFICATION_JOB)
                .arg_param(&["id"])
                .completion_cb("id", pbs_config::verify::complete_verification_job_id),
        )
        .insert(
            "remove",
            CliCommand::new(&api2::config::verify::API_METHOD_DELETE_VERIFICATION_JOB)
                .arg_param(&["id"])
                .completion_cb("id", pbs_config::verify::complete_verification_job_id),
        );

    cmd_def.into()
}
