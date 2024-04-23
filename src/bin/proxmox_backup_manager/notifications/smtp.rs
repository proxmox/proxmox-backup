use anyhow::Error;
use proxmox_notify::schema::ENTITY_NAME_SCHEMA;
use serde_json::Value;

use proxmox_router::{cli::*, ApiHandler, RpcEnvironment};
use proxmox_schema::api;

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
/// List all endpoints.
fn list_endpoints(param: Value, rpcenv: &mut dyn RpcEnvironment) -> Result<Value, Error> {
    let output_format = get_output_format(&param);

    let info = &api2::config::notifications::smtp::API_METHOD_LIST_ENDPOINTS;
    let mut data = match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    let options = default_table_format_options()
        .column(ColumnConfig::new("disable"))
        .column(ColumnConfig::new("name"))
        .column(ColumnConfig::new("server"))
        .column(ColumnConfig::new("from-address"))
        .column(ColumnConfig::new("mailto"))
        .column(ColumnConfig::new("mailto-user"))
        .column(ColumnConfig::new("comment"));

    format_and_print_result_full(&mut data, &info.returns, &output_format, &options);

    Ok(Value::Null)
}

#[api(
    input: {
        properties: {
            name: {
                schema: ENTITY_NAME_SCHEMA,
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        }
    }
)]
/// Show a single endpoint.
fn show_endpoint(param: Value, rpcenv: &mut dyn RpcEnvironment) -> Result<Value, Error> {
    let output_format = get_output_format(&param);

    let info = &api2::config::notifications::smtp::API_METHOD_GET_ENDPOINT;
    let mut data = match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    let options = default_table_format_options();
    format_and_print_result_full(&mut data, &info.returns, &output_format, &options);

    Ok(Value::Null)
}

pub fn commands() -> CommandLineInterface {
    let cmd_def = CliCommandMap::new()
        .insert("list", CliCommand::new(&API_METHOD_LIST_ENDPOINTS))
        .insert(
            "show",
            CliCommand::new(&API_METHOD_SHOW_ENDPOINT).arg_param(&["name"]),
        )
        .insert(
            "create",
            CliCommand::new(&api2::config::notifications::smtp::API_METHOD_ADD_ENDPOINT)
                .arg_param(&["name"]),
        )
        .insert(
            "update",
            CliCommand::new(&api2::config::notifications::smtp::API_METHOD_UPDATE_ENDPOINT)
                .arg_param(&["name"]),
        )
        .insert(
            "delete",
            CliCommand::new(&api2::config::notifications::smtp::API_METHOD_DELETE_ENDPOINT)
                .arg_param(&["name"]),
        );
    cmd_def.into()
}
