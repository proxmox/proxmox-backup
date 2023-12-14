use anyhow::{bail, Error};
use serde_json::Value;

use proxmox_router::{cli::*, ApiHandler, RpcEnvironment};
use proxmox_schema::api;
use proxmox_section_config::SectionConfigData;

use pbs_config::drive::{complete_changer_name, complete_drive_name};

use pbs_api_types::CHANGER_NAME_SCHEMA;

use pbs_tape::linux_list_drives::complete_changer_path;

use proxmox_backup::{api2, tape::drive::media_changer};

pub fn lookup_changer_name(param: &Value, config: &SectionConfigData) -> Result<String, Error> {
    if let Some(name) = param["name"].as_str() {
        return Ok(String::from(name));
    }

    let mut empty = Value::Null;

    if let Ok(drive) = crate::extract_drive_name(&mut empty, config) {
        if let Ok(Some((_, name))) = media_changer(config, &drive) {
            return Ok(name);
        }
    }

    bail!("unable to get (default) changer name");
}

pub fn changer_commands() -> CommandLineInterface {
    let cmd_def = CliCommandMap::new()
        .insert("scan", CliCommand::new(&API_METHOD_SCAN_FOR_CHANGERS))
        .insert("list", CliCommand::new(&API_METHOD_LIST_CHANGERS))
        .insert(
            "config",
            CliCommand::new(&API_METHOD_GET_CONFIG)
                .arg_param(&["name"])
                .completion_cb("name", complete_changer_name),
        )
        .insert(
            "remove",
            CliCommand::new(&api2::config::changer::API_METHOD_DELETE_CHANGER)
                .arg_param(&["name"])
                .completion_cb("name", complete_changer_name),
        )
        .insert(
            "create",
            CliCommand::new(&api2::config::changer::API_METHOD_CREATE_CHANGER)
                .arg_param(&["name"])
                .completion_cb("name", complete_drive_name)
                .completion_cb("path", complete_changer_path),
        )
        .insert(
            "update",
            CliCommand::new(&api2::config::changer::API_METHOD_UPDATE_CHANGER)
                .arg_param(&["name"])
                .completion_cb("name", complete_changer_name)
                .completion_cb("path", complete_changer_path),
        )
        .insert(
            "status",
            CliCommand::new(&API_METHOD_GET_STATUS)
                .arg_param(&["name"])
                .completion_cb("name", complete_changer_name),
        )
        .insert(
            "transfer",
            CliCommand::new(&API_METHOD_TRANSFER)
                .arg_param(&["name"])
                .completion_cb("name", complete_changer_name),
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
/// List changers
fn list_changers(param: Value, rpcenv: &mut dyn RpcEnvironment) -> Result<(), Error> {
    let output_format = get_output_format(&param);
    let info = &api2::tape::changer::API_METHOD_LIST_CHANGERS;
    let mut data = match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    let options = default_table_format_options()
        .column(ColumnConfig::new("name"))
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
        },
    },
)]
/// Scan for SCSI tape changers
fn scan_for_changers(param: Value, rpcenv: &mut dyn RpcEnvironment) -> Result<(), Error> {
    let output_format = get_output_format(&param);
    let info = &api2::tape::API_METHOD_SCAN_CHANGERS;
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
                schema: CHANGER_NAME_SCHEMA,
            },
        },
    },
)]
/// Get tape changer configuration
fn get_config(param: Value, rpcenv: &mut dyn RpcEnvironment) -> Result<(), Error> {
    let output_format = get_output_format(&param);
    let info = &api2::config::changer::API_METHOD_GET_CONFIG;
    let mut data = match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    let options = default_table_format_options()
        .column(ColumnConfig::new("name"))
        .column(ColumnConfig::new("path"))
        .column(ColumnConfig::new("eject-before-unload"))
        .column(ColumnConfig::new("export-slots"));

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
                optional: true,
            },
            cache: {
                description: "Use cached value.",
                type: bool,
                optional: true,
                default: true,
            },
        },
    },
)]
/// Get tape changer status
async fn get_status(mut param: Value, rpcenv: &mut dyn RpcEnvironment) -> Result<(), Error> {
    let (config, _digest) = pbs_config::drive::config()?;

    param["name"] = lookup_changer_name(&param, &config)?.into();

    let output_format = get_output_format(&param);
    let info = &api2::tape::changer::API_METHOD_GET_STATUS;
    let mut data = match info.handler {
        ApiHandler::Async(handler) => (handler)(param, info, rpcenv).await?,
        _ => unreachable!(),
    };

    let render_label_text = |value: &Value, _record: &Value| -> Result<String, Error> {
        if value.is_null() {
            return Ok(String::new());
        }
        let text = value.as_str().unwrap().to_string();
        if text.is_empty() {
            Ok(String::from("--FULL--"))
        } else {
            Ok(text)
        }
    };

    let options = default_table_format_options()
        .sortby("entry-kind", false)
        .sortby("entry-id", false)
        .column(ColumnConfig::new("entry-kind"))
        .column(ColumnConfig::new("entry-id"))
        .column(ColumnConfig::new("label-text").renderer(render_label_text))
        .column(ColumnConfig::new("loaded-slot"));

    format_and_print_result_full(&mut data, &info.returns, &output_format, &options);

    Ok(())
}

#[api(
    input: {
        properties: {
            name: {
                schema: CHANGER_NAME_SCHEMA,
                optional: true,
            },
            from: {
                description: "Source slot number",
                type: u64,
                minimum: 1,
            },
            to: {
                description: "Destination slot number",
                type: u64,
                minimum: 1,
            },
        },
    },
)]
/// Transfers media from one slot to another
pub async fn transfer(mut param: Value, rpcenv: &mut dyn RpcEnvironment) -> Result<(), Error> {
    let (config, _digest) = pbs_config::drive::config()?;

    param["name"] = lookup_changer_name(&param, &config)?.into();

    let info = &api2::tape::changer::API_METHOD_TRANSFER;
    match info.handler {
        ApiHandler::Async(handler) => (handler)(param, info, rpcenv).await?,
        _ => unreachable!(),
    };

    Ok(())
}
