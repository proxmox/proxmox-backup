use anyhow::{Error};
use serde_json::Value;

use proxmox::{
    api::{
        api,
        cli::*,
        RpcEnvironment,
        ApiHandler,
    },
};

use proxmox_backup::{
    api2::{
        self,
        types::{
            CHANGER_NAME_SCHEMA,
        },
    },
    tape::{
        complete_changer_path,
    },
    config::{
        drive::{
            complete_drive_name,
            complete_changer_name,
        }
    },
};

pub fn changer_commands() -> CommandLineInterface {

    let cmd_def = CliCommandMap::new()
        .insert("scan", CliCommand::new(&API_METHOD_SCAN_FOR_CHANGERS))
        .insert("list", CliCommand::new(&API_METHOD_LIST_CHANGERS))
        .insert("config",
                CliCommand::new(&API_METHOD_GET_CONFIG)
                .arg_param(&["name"])
                .completion_cb("name", complete_changer_name)
        )
        .insert(
            "remove",
            CliCommand::new(&api2::config::changer::API_METHOD_DELETE_CHANGER)
                .arg_param(&["name"])
                .completion_cb("name", complete_changer_name)
        )
        .insert(
            "create",
            CliCommand::new(&api2::config::changer::API_METHOD_CREATE_CHANGER)
                .arg_param(&["name"])
                .completion_cb("name", complete_drive_name)
                .completion_cb("path", complete_changer_path)
        )
        .insert(
            "update",
            CliCommand::new(&api2::config::changer::API_METHOD_UPDATE_CHANGER)
                .arg_param(&["name"])
                .completion_cb("name", complete_changer_name)
                .completion_cb("path", complete_changer_path)
        )
        .insert("status",
                CliCommand::new(&API_METHOD_GET_STATUS)
                .arg_param(&["name"])
                .completion_cb("name", complete_changer_name)
        )
        .insert("transfer",
                CliCommand::new(&api2::tape::changer::API_METHOD_TRANSFER)
                .arg_param(&["name"])
                .completion_cb("name", complete_changer_name)
        )
        ;

    cmd_def.into()
}

#[api(
    input: {
        properties: {
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        },
    },
)]
/// List changers
fn list_changers(
    param: Value,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {

    let output_format = get_output_format(&param);
    let info = &api2::config::changer::API_METHOD_LIST_CHANGERS;
    let mut data = match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    let options = default_table_format_options()
        .column(ColumnConfig::new("name"))
        .column(ColumnConfig::new("path"))
        .column(ColumnConfig::new("vendor"))
        .column(ColumnConfig::new("model"))
        .column(ColumnConfig::new("serial"))
        ;

    format_and_print_result_full(&mut data, &info.returns, &output_format, &options);

    Ok(())
}

#[api(
    input: {
        properties: {
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        },
    },
)]
/// Scan for SCSI tape changers
fn scan_for_changers(
    param: Value,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {

    let output_format = get_output_format(&param);
    let info = &api2::tape::changer::API_METHOD_SCAN_CHANGERS;
    let mut data = match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    let options = default_table_format_options()
        .column(ColumnConfig::new("path"))
        .column(ColumnConfig::new("vendor"))
        .column(ColumnConfig::new("model"))
        .column(ColumnConfig::new("serial"))
        ;

    format_and_print_result_full(&mut data, &info.returns, &output_format, &options);

    Ok(())
}

#[api(
    input: {
        properties: {
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
            name: {
                schema: CHANGER_NAME_SCHEMA,
            },
        },
    },
)]
/// Get tape changer configuration
fn get_config(
    param: Value,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {

    let output_format = get_output_format(&param);
    let info = &api2::config::changer::API_METHOD_GET_CONFIG;
    let mut data = match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    let options = default_table_format_options()
        .column(ColumnConfig::new("name"))
        .column(ColumnConfig::new("path"))
        ;

    format_and_print_result_full(&mut data, &info.returns, &output_format, &options);

    Ok(())
}

#[api(
    input: {
        properties: {
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
             name: {
                schema: CHANGER_NAME_SCHEMA,
            },
        },
    },
)]
/// Get tape changer status
async fn get_status(
    param: Value,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {

    let output_format = get_output_format(&param);
    let info = &api2::tape::changer::API_METHOD_GET_STATUS;
    let mut data = match info.handler {
        ApiHandler::Async(handler) => (handler)(param, info, rpcenv).await?,
        _ => unreachable!(),
    };

    let options = default_table_format_options()
        .column(ColumnConfig::new("entry-kind"))
        .column(ColumnConfig::new("entry-id"))
        .column(ColumnConfig::new("changer-id"))
        .column(ColumnConfig::new("loaded-slot"))
        ;

    format_and_print_result_full(&mut data, &info.returns, &output_format, &options);

    Ok(())
}
