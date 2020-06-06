use anyhow::Error;
use serde_json::Value;

use proxmox::api::{api, cli::*, RpcEnvironment, ApiHandler};

use proxmox_backup::tools::disks::*;
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
/// Local disk list.
fn list_disks(mut param: Value, rpcenv: &mut dyn RpcEnvironment) -> Result<Value, Error> {

    let output_format = get_output_format(&param);

    param["node"] = "localhost".into();

    let info = &api2::node::disks::API_METHOD_LIST_DISKS;
    let mut data = match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    let options = default_table_format_options()
        .column(ColumnConfig::new("name"))
        .column(ColumnConfig::new("used"))
        .column(ColumnConfig::new("disk-type"))
        .column(ColumnConfig::new("size"))
        .column(ColumnConfig::new("model"))
        .column(ColumnConfig::new("wearout"))
        .column(ColumnConfig::new("status"))
        ;

    format_and_print_result_full(&mut data, info.returns, &output_format, &options);

    Ok(Value::Null)
}

#[api(
    input: {
        properties: {
            disk: {
		description: "Block device name.",
		type: String,
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        }
    },
    returns: {
        description: "SMART attributes.",
        type: Array,
        items: {
            type: SmartAttribute,
        },
    }
)]
/// Show SMART attributes.
fn smart_attributes(mut param: Value, rpcenv: &mut dyn RpcEnvironment) -> Result<Value, Error> {

    let output_format = get_output_format(&param);

    param["node"] = "localhost".into();

    let info = &api2::node::disks::API_METHOD_SMART_STATUS;
    let mut data = match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    let mut data = data["attributes"].take();

    let options = default_table_format_options();
    format_and_print_result_full(&mut data, API_METHOD_SMART_ATTRIBUTES.returns, &output_format, &options);

    Ok(Value::Null)
}

pub fn disk_commands() -> CommandLineInterface {

    let cmd_def = CliCommandMap::new()
        .insert("list", CliCommand::new(&API_METHOD_LIST_DISKS))
        .insert("smart-attributes",
                CliCommand::new(&API_METHOD_SMART_ATTRIBUTES)
                .arg_param(&["disk"])
                .completion_cb("disk", complete_disk_name)
        );

    cmd_def.into()
}
