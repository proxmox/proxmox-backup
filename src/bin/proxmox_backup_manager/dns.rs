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
/// Read DNS settings
fn get_dns(mut param: Value, rpcenv: &mut dyn RpcEnvironment) -> Result<Value, Error> {
    let output_format = get_output_format(&param);

    param["node"] = "localhost".into();

    let info = &api2::node::dns::API_METHOD_GET_DNS;
    let mut data = match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    let options = default_table_format_options()
        .column(ColumnConfig::new("search"))
        .column(ColumnConfig::new("dns1"))
        .column(ColumnConfig::new("dns2"))
        .column(ColumnConfig::new("dns3"));

    format_and_print_result_full(&mut data, &info.returns, &output_format, &options);

    Ok(Value::Null)
}

pub fn dns_commands() -> CommandLineInterface {
    let cmd_def = CliCommandMap::new()
        .insert("get", CliCommand::new(&API_METHOD_GET_DNS))
        .insert(
            "set",
            CliCommand::new(&api2::node::dns::API_METHOD_UPDATE_DNS)
                .fixed_param("node", String::from("localhost")),
        );

    cmd_def.into()
}
