use anyhow::Error;
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
            DRIVE_ID_SCHEMA,
            CHANGER_ID_SCHEMA,
            LINUX_DRIVE_PATH_SCHEMA,
        },
    },
    tape::{
        complete_drive_path,
    },
    config::drive::{
        complete_drive_name,
        complete_changer_name,
        complete_linux_drive_name,
    },
};

pub fn drive_commands() -> CommandLineInterface {

    let cmd_def = CliCommandMap::new()
        .insert("scan", CliCommand::new(&API_METHOD_SCAN_FOR_DRIVES))
        .insert("list", CliCommand::new(&API_METHOD_LIST_DRIVES))
        .insert("config",
                CliCommand::new(&API_METHOD_GET_CONFIG)
                .arg_param(&["name"])
                .completion_cb("name", complete_linux_drive_name)
        )
        .insert(
            "remove",
            CliCommand::new(&API_METHOD_DELETE_DRIVE)
                .arg_param(&["name"])
                .completion_cb("name", complete_linux_drive_name)
        )
        .insert(
            "create",
            CliCommand::new(&API_METHOD_CREATE_LINUX_DRIVE)
                .arg_param(&["name"])
                .completion_cb("name", complete_drive_name)
                .completion_cb("path", complete_drive_path)
                .completion_cb("changer", complete_changer_name)
        )
        .insert(
            "update",
            CliCommand::new(&API_METHOD_UPDATE_LINUX_DRIVE)
                .arg_param(&["name"])
                .completion_cb("name", complete_linux_drive_name)
                .completion_cb("path", complete_drive_path)
                .completion_cb("changer", complete_changer_name)
        )
        .insert(
            "load",
            CliCommand::new(&API_METHOD_LOAD_SLOT)
                .arg_param(&["name"])
                .completion_cb("name", complete_linux_drive_name)
        )
        .insert(
            "unload",
            CliCommand::new(&API_METHOD_UNLOAD)
                .arg_param(&["name"])
                .completion_cb("name", complete_linux_drive_name)
        )
        ;

    cmd_def.into()
}

#[api(
    input: {
        properties: {
            name: {
                schema: DRIVE_ID_SCHEMA,
            },
            path: {
                schema: LINUX_DRIVE_PATH_SCHEMA,
            },
            changer: {
                schema: CHANGER_ID_SCHEMA,
                optional: true,
            },
        },
    },
)]
/// Create a new drive
fn create_linux_drive(
    param: Value,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {

    let info = &api2::config::drive::API_METHOD_CREATE_DRIVE;
    match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

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
/// List drives
fn list_drives(
    param: Value,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {

    let output_format = get_output_format(&param);
    let info = &api2::config::drive::API_METHOD_LIST_DRIVES;
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
        .column(ColumnConfig::new("serial"))
        ;

    format_and_print_result_full(&mut data, info.returns, &output_format, &options);

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
fn scan_for_drives(
    param: Value,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {

    let output_format = get_output_format(&param);
    let info = &api2::tape::drive::API_METHOD_SCAN_DRIVES;
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

    format_and_print_result_full(&mut data, info.returns, &output_format, &options);

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
                schema: DRIVE_ID_SCHEMA,
            },
        },
    },
)]
/// Get pool configuration
fn get_config(
    param: Value,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {

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
        ;

    format_and_print_result_full(&mut data, info.returns, &output_format, &options);

    Ok(())
}

#[api(
    input: {
        properties: {
            name: {
                schema: DRIVE_ID_SCHEMA,
            },
        },
    },
)]
/// Delete a drive configuration
fn delete_drive(
    param: Value,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {

    let info = &api2::config::drive::API_METHOD_DELETE_DRIVE;

    match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    Ok(())
}

#[api(
    input: {
        properties: {
            name: {
                schema: DRIVE_ID_SCHEMA,
            },
            path: {
                schema: LINUX_DRIVE_PATH_SCHEMA,
                optional: true,
            },
            changer: {
                schema: CHANGER_ID_SCHEMA,
                optional: true,
            },
        },
    },
)]
/// Update a drive configuration
fn update_linux_drive(
    param: Value,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {

    let info = &api2::config::drive::API_METHOD_UPDATE_DRIVE;

    match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    Ok(())
}

#[api(
    input: {
        properties: {
            name: {
                schema: DRIVE_ID_SCHEMA,
            },
            slot: {
                type: u64,
                description: "Source slot number",
                minimum: 1,
            },
        },
    },
)]
/// Load media via changer from slot
fn load_slot(
    param: Value,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {

    let info = &api2::tape::drive::API_METHOD_LOAD_SLOT;

    match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    Ok(())
}

#[api(
    input: {
        properties: {
            name: {
                schema: DRIVE_ID_SCHEMA,
            },
            slot: {
                description: "Target slot number. If omitted, defaults to the slot that the drive was loaded from.",
                type: u64,
                minimum: 1,
                optional: true,
            },
        },
    },
)]
/// Unload media via changer
fn unload(
    param: Value,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {

    let info = &api2::tape::drive::API_METHOD_UNLOAD;

    match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    Ok(())
}
