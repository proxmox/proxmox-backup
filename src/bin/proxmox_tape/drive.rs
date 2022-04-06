use anyhow::Error;
use serde_json::Value;

use proxmox_router::{cli::*, ApiHandler, RpcEnvironment};
use proxmox_schema::api;

use pbs_api_types::DRIVE_NAME_SCHEMA;

use pbs_config::drive::{complete_changer_name, complete_drive_name, complete_lto_drive_name};

use pbs_tape::linux_list_drives::complete_drive_path;

use proxmox_backup::api2;

pub fn drive_commands() -> CommandLineInterface {
    let cmd_def = CliCommandMap::new()
        .insert("scan", CliCommand::new(&API_METHOD_SCAN_FOR_DRIVES))
        .insert("list", CliCommand::new(&API_METHOD_LIST_DRIVES))
        .insert(
            "config",
            CliCommand::new(&API_METHOD_GET_CONFIG)
                .arg_param(&["name"])
                .completion_cb("name", complete_lto_drive_name),
        )
        .insert(
            "remove",
            CliCommand::new(&api2::config::drive::API_METHOD_DELETE_DRIVE)
                .arg_param(&["name"])
                .completion_cb("name", complete_lto_drive_name),
        )
        .insert(
            "create",
            CliCommand::new(&api2::config::drive::API_METHOD_CREATE_DRIVE)
                .arg_param(&["name"])
                .completion_cb("name", complete_drive_name)
                .completion_cb("path", complete_drive_path)
                .completion_cb("changer", complete_changer_name),
        )
        .insert(
            "update",
            CliCommand::new(&api2::config::drive::API_METHOD_UPDATE_DRIVE)
                .arg_param(&["name"])
                .completion_cb("name", complete_lto_drive_name)
                .completion_cb("path", complete_drive_path)
                .completion_cb("changer", complete_changer_name),
        );

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
/// List drives
fn list_drives(param: Value, rpcenv: &mut dyn RpcEnvironment) -> Result<(), Error> {
    let output_format = get_output_format(&param);
    let info = &api2::tape::drive::API_METHOD_LIST_DRIVES;
    let mut data = match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    let options = default_table_format_options()
        .column(ColumnConfig::new("name"))
        .column(ColumnConfig::new("path"))
        .column(ColumnConfig::new("changer"))
        .column(ColumnConfig::new("vendor"))
        .column(ColumnConfig::new("model"))
        .column(ColumnConfig::new("serial"));

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
    }
)]
/// Scan for drives
fn scan_for_drives(param: Value, rpcenv: &mut dyn RpcEnvironment) -> Result<(), Error> {
    let output_format = get_output_format(&param);
    let info = &api2::tape::API_METHOD_SCAN_DRIVES;
    let mut data = match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    let options = default_table_format_options()
        .column(ColumnConfig::new("path"))
        .column(ColumnConfig::new("vendor"))
        .column(ColumnConfig::new("model"))
        .column(ColumnConfig::new("serial"));

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
                schema: DRIVE_NAME_SCHEMA,
            },
        },
    },
)]
/// Get pool configuration
fn get_config(param: Value, rpcenv: &mut dyn RpcEnvironment) -> Result<(), Error> {
    let output_format = get_output_format(&param);
    let info = &api2::config::drive::API_METHOD_GET_CONFIG;
    let mut data = match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    let options = default_table_format_options()
        .column(ColumnConfig::new("name"))
        .column(ColumnConfig::new("path"))
        .column(ColumnConfig::new("changer"))
        .column(ColumnConfig::new("changer-drivenum"));

    format_and_print_result_full(&mut data, &info.returns, &output_format, &options);

    Ok(())
}
