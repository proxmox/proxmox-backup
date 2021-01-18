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
    },
    config::tape_encryption_keys::complete_key_fingerprint,
};

pub fn encryption_key_commands() -> CommandLineInterface {

    let cmd_def = CliCommandMap::new()
        .insert("list", CliCommand::new(&API_METHOD_LIST_KEYS))
        .insert(
            "create",
            CliCommand::new(&api2::config::tape_encryption_keys::API_METHOD_CREATE_KEY)
        )
        .insert(
            "remove",
            CliCommand::new(&api2::config::tape_encryption_keys::API_METHOD_DELETE_KEY)
                .arg_param(&["fingerprint"])
                .completion_cb("fingerprint", complete_key_fingerprint)
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
/// List keys
fn list_keys(
    param: Value,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {

    let output_format = get_output_format(&param);
    let info = &api2::config::tape_encryption_keys::API_METHOD_LIST_KEYS;
    let mut data = match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    let options = default_table_format_options()
        .column(ColumnConfig::new("fingerprint"))
        .column(ColumnConfig::new("hint"))
        ;

    format_and_print_result_full(&mut data, &info.returns, &output_format, &options);

    Ok(())
}
