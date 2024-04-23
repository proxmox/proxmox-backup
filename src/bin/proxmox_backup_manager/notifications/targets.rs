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
/// List targets.
fn list_targets(param: Value, rpcenv: &mut dyn RpcEnvironment) -> Result<Value, Error> {
    let output_format = get_output_format(&param);

    let info = &api2::config::notifications::targets::API_METHOD_LIST_TARGETS;
    let mut data = match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    let options = default_table_format_options()
        .column(ColumnConfig::new("disable"))
        .column(ColumnConfig::new("name"))
        .column(ColumnConfig::new("type"))
        .column(ColumnConfig::new("origin"))
        .column(ColumnConfig::new("comment"));

    format_and_print_result_full(&mut data, &info.returns, &output_format, &options);

    Ok(Value::Null)
}

pub fn commands() -> CommandLineInterface {
    let cmd_def = CliCommandMap::new()
        .insert("list", CliCommand::new(&API_METHOD_LIST_TARGETS))
        .insert(
            "test",
            CliCommand::new(&api2::config::notifications::targets::API_METHOD_TEST_TARGET)
                .arg_param(&["name"]),
        );

    cmd_def.into()
}
