use anyhow::Error;
use serde_json::Value;

use proxmox_router::{cli::*, ApiHandler, RpcEnvironment};
use proxmox_schema::api;

use pbs_client::view_task_result;
use pbs_api_types::{DataStoreConfig, DATASTORE_SCHEMA};

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
/// Datastore list.
fn list_datastores(param: Value, rpcenv: &mut dyn RpcEnvironment) -> Result<Value, Error> {

    let output_format = get_output_format(&param);

    let info = &api2::config::datastore::API_METHOD_LIST_DATASTORES;
    let mut data = match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    let options = default_table_format_options()
        .column(ColumnConfig::new("name"))
        .column(ColumnConfig::new("path"))
        .column(ColumnConfig::new("comment"));

    format_and_print_result_full(&mut data, &info.returns, &output_format, &options);

    Ok(Value::Null)
}

#[api(
    input: {
        properties: {
            name: {
                schema: DATASTORE_SCHEMA,
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        }
    }
)]
/// Show datastore configuration
fn show_datastore(param: Value, rpcenv: &mut dyn RpcEnvironment) -> Result<Value, Error> {

    let output_format = get_output_format(&param);

    let info = &api2::config::datastore::API_METHOD_READ_DATASTORE;
    let mut data = match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    let options = default_table_format_options();
    format_and_print_result_full(&mut data, &info.returns, &output_format, &options);

    Ok(Value::Null)
}

#[api(
    protected: true,
    input: {
        properties: {
            config: {
                type: DataStoreConfig,
                flatten: true,
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        },
    },
)]
/// Create new datastore config.
async fn create_datastore(mut param: Value) -> Result<Value, Error> {

    let output_format = extract_output_format(&mut param);

    let mut client = connect_to_localhost()?;

    let result = client.post("api2/json/config/datastore", Some(param)).await?;

    view_task_result(&mut client, result, &output_format).await?;

    Ok(Value::Null)
}

pub fn datastore_commands() -> CommandLineInterface {

    let cmd_def = CliCommandMap::new()
        .insert("list", CliCommand::new(&API_METHOD_LIST_DATASTORES))
        .insert("show",
                CliCommand::new(&API_METHOD_SHOW_DATASTORE)
                .arg_param(&["name"])
                .completion_cb("name", pbs_config::datastore::complete_datastore_name)
        )
        .insert("create",
                CliCommand::new(&API_METHOD_CREATE_DATASTORE)
                .arg_param(&["name", "path"])
        )
        .insert("update",
                CliCommand::new(&api2::config::datastore::API_METHOD_UPDATE_DATASTORE)
                .arg_param(&["name"])
                .completion_cb("name", pbs_config::datastore::complete_datastore_name)
                .completion_cb("gc-schedule", pbs_config::datastore::complete_calendar_event)
                .completion_cb("prune-schedule", pbs_config::datastore::complete_calendar_event)
        )
        .insert("remove",
                CliCommand::new(&api2::config::datastore::API_METHOD_DELETE_DATASTORE)
                .arg_param(&["name"])
                .completion_cb("name", pbs_config::datastore::complete_datastore_name)
        );

    cmd_def.into()
}
