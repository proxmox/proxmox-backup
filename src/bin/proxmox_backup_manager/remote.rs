use anyhow::Error;
use serde_json::Value;

use proxmox_router::{cli::*, ApiHandler, RpcEnvironment};
use proxmox_schema::api;

use pbs_api_types::REMOTE_ID_SCHEMA;

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
/// List configured remotes.
fn list_remotes(param: Value, rpcenv: &mut dyn RpcEnvironment) -> Result<Value, Error> {
    let output_format = get_output_format(&param);

    let info = &api2::config::remote::API_METHOD_LIST_REMOTES;
    let mut data = match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    let options = default_table_format_options()
        .column(ColumnConfig::new("name"))
        .column(ColumnConfig::new("host"))
        .column(ColumnConfig::new("auth-id"))
        .column(ColumnConfig::new("fingerprint"))
        .column(ColumnConfig::new("comment"));

    format_and_print_result_full(&mut data, &info.returns, &output_format, &options);

    Ok(Value::Null)
}

#[api(
    input: {
        properties: {
            name: {
                schema: REMOTE_ID_SCHEMA,
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        }
    }
)]
/// Show remote configuration
fn show_remote(param: Value, rpcenv: &mut dyn RpcEnvironment) -> Result<Value, Error> {
    let output_format = get_output_format(&param);

    let info = &api2::config::remote::API_METHOD_READ_REMOTE;
    let mut data = match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    let options = default_table_format_options();
    format_and_print_result_full(&mut data, &info.returns, &output_format, &options);

    Ok(Value::Null)
}

pub fn remote_commands() -> CommandLineInterface {
    let cmd_def = CliCommandMap::new()
        .insert("list", CliCommand::new(&API_METHOD_LIST_REMOTES))
        .insert(
            "show",
            CliCommand::new(&API_METHOD_SHOW_REMOTE)
                .arg_param(&["name"])
                .completion_cb("name", pbs_config::remote::complete_remote_name),
        )
        .insert(
            "create",
            // fixme: howto handle password parameter?
            CliCommand::new(&api2::config::remote::API_METHOD_CREATE_REMOTE).arg_param(&["name"]),
        )
        .insert(
            "update",
            CliCommand::new(&api2::config::remote::API_METHOD_UPDATE_REMOTE)
                .arg_param(&["name"])
                .completion_cb("name", pbs_config::remote::complete_remote_name),
        )
        .insert(
            "remove",
            CliCommand::new(&api2::config::remote::API_METHOD_DELETE_REMOTE)
                .arg_param(&["name"])
                .completion_cb("name", pbs_config::remote::complete_remote_name),
        );

    cmd_def.into()
}
