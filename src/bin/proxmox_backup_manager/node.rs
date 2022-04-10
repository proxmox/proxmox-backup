use anyhow::Error;
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
/// Show node configuration
fn get_node_config(param: Value, rpcenv: &mut dyn RpcEnvironment) -> Result<Value, Error> {
    let output_format = get_output_format(&param);

    let info = &api2::node::config::API_METHOD_GET_NODE_CONFIG;
    let mut data = match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    let options = default_table_format_options();
    format_and_print_result_full(&mut data, &info.returns, &output_format, &options);

    Ok(Value::Null)
}

pub fn node_commands() -> CommandLineInterface {
    let cmd_def = CliCommandMap::new()
        .insert("show", CliCommand::new(&API_METHOD_GET_NODE_CONFIG))
        .insert(
            "update",
            CliCommand::new(&api2::node::config::API_METHOD_UPDATE_NODE_CONFIG)
                .fixed_param("node", String::from("localhost")),
        );

    cmd_def.into()
}
