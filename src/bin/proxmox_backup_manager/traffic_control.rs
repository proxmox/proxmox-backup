use anyhow::Error;
use serde_json::Value;

use proxmox_router::{cli::*, ApiHandler, RpcEnvironment};
use proxmox_schema::api;

use pbs_api_types::TRAFFIC_CONTROL_ID_SCHEMA;
use pbs_tools::format::render_bytes_human_readable;

use proxmox_backup::api2;
use proxmox_backup::client_helpers::connect_to_localhost;

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
/// List configured traffic control rules.
fn list_traffic_controls(param: Value, rpcenv: &mut dyn RpcEnvironment) -> Result<Value, Error> {
    let output_format = get_output_format(&param);

    let info = &api2::config::traffic_control::API_METHOD_LIST_TRAFFIC_CONTROLS;
    let mut data = match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    let options = default_table_format_options()
        .column(ColumnConfig::new("name"))
        .column(ColumnConfig::new("rate-in"))
        .column(ColumnConfig::new("burst-in"))
        .column(ColumnConfig::new("rate-out"))
        .column(ColumnConfig::new("burst-out"))
        .column(ColumnConfig::new("network"))
        .column(ColumnConfig::new("timeframe"))
        .column(ColumnConfig::new("comment"));

    format_and_print_result_full(&mut data, &info.returns, &output_format, &options);

    Ok(Value::Null)
}

#[api(
    input: {
        properties: {
            name: {
                schema: TRAFFIC_CONTROL_ID_SCHEMA,
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        }
    }
)]
/// Show traffic control configuration
fn show_traffic_control(param: Value, rpcenv: &mut dyn RpcEnvironment) -> Result<Value, Error> {
    let output_format = get_output_format(&param);

    let info = &api2::config::traffic_control::API_METHOD_READ_TRAFFIC_CONTROL;
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
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        }
    }
)]
/// Show current traffic for all rules.
async fn show_current_traffic(param: Value) -> Result<Value, Error> {
    let output_format = get_output_format(&param);

    let client = connect_to_localhost()?;

    let mut result = client.get("api2/json/admin/traffic-control", None).await?;

    let mut data = result["data"].take();

    let info = &api2::admin::traffic_control::API_METHOD_SHOW_CURRENT_TRAFFIC;

    let options = default_table_format_options()
        .column(ColumnConfig::new("name"))
        .column(ColumnConfig::new("cur-rate-in").renderer(render_bytes_human_readable))
        .column(ColumnConfig::new("cur-rate-out").renderer(render_bytes_human_readable));

    format_and_print_result_full(&mut data, &info.returns, &output_format, &options);

    Ok(Value::Null)
}

pub fn traffic_control_commands() -> CommandLineInterface {
    let cmd_def = CliCommandMap::new()
        .insert("list", CliCommand::new(&API_METHOD_LIST_TRAFFIC_CONTROLS))
        .insert("traffic", CliCommand::new(&API_METHOD_SHOW_CURRENT_TRAFFIC))
        .insert(
            "show",
            CliCommand::new(&API_METHOD_SHOW_TRAFFIC_CONTROL)
                .arg_param(&["name"])
                .completion_cb(
                    "name",
                    pbs_config::traffic_control::complete_traffic_control_name,
                ),
        )
        .insert(
            "create",
            CliCommand::new(&api2::config::traffic_control::API_METHOD_CREATE_TRAFFIC_CONTROL)
                .arg_param(&["name"]),
        )
        .insert(
            "update",
            CliCommand::new(&api2::config::traffic_control::API_METHOD_UPDATE_TRAFFIC_CONTROL)
                .arg_param(&["name"])
                .completion_cb(
                    "name",
                    pbs_config::traffic_control::complete_traffic_control_name,
                ),
        )
        .insert(
            "remove",
            CliCommand::new(&api2::config::traffic_control::API_METHOD_DELETE_TRAFFIC_CONTROL)
                .arg_param(&["name"])
                .completion_cb(
                    "name",
                    pbs_config::traffic_control::complete_traffic_control_name,
                ),
        );

    cmd_def.into()
}
