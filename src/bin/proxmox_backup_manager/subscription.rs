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
/// Read subscription info.
fn get(param: Value, rpcenv: &mut dyn RpcEnvironment) -> Result<Value, Error> {
    let output_format = get_output_format(&param);

    let info = &api2::node::subscription::API_METHOD_GET_SUBSCRIPTION;
    let mut data = match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    let options = default_table_format_options();
    format_and_print_result_full(&mut data, &info.returns, &output_format, &options);

    Ok(Value::Null)
}

pub fn subscription_commands() -> CommandLineInterface {
    let cmd_def = CliCommandMap::new()
        .insert("get", CliCommand::new(&API_METHOD_GET))
        .insert(
            "set",
            CliCommand::new(&api2::node::subscription::API_METHOD_SET_SUBSCRIPTION)
                .fixed_param("node", "localhost".into())
                .arg_param(&["key"]),
        )
        .insert(
            "update",
            CliCommand::new(&api2::node::subscription::API_METHOD_CHECK_SUBSCRIPTION)
                .fixed_param("node", "localhost".into()),
        )
        .insert(
            "remove",
            CliCommand::new(&api2::node::subscription::API_METHOD_DELETE_SUBSCRIPTION)
                .fixed_param("node", "localhost".into()),
        );

    cmd_def.into()
}
