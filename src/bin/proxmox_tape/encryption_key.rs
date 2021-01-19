use anyhow::{bail, Error};
use serde_json::Value;

use proxmox::{
    api::{
        api,
        cli::*,
        RpcEnvironment,
        ApiHandler,
    },
    sys::linux::tty,
};

use proxmox_backup::{
    config,
    api2::{
        self,
        types::{
            DRIVE_NAME_SCHEMA,
        },
    },
    config::tape_encryption_keys::complete_key_fingerprint,
};

pub fn encryption_key_commands() -> CommandLineInterface {

    let cmd_def = CliCommandMap::new()
        .insert("list", CliCommand::new(&API_METHOD_LIST_KEYS))
        .insert(
            "create",
            CliCommand::new(&API_METHOD_CREATE_KEY)
        )
        .insert(
            "restore",
            CliCommand::new(&API_METHOD_RESTORE_KEY)
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
            drive: {
                schema: DRIVE_NAME_SCHEMA,
                optional: true,
            },
        },
    },
)]
/// Restore encryption key from tape (read password from stdin)
async fn restore_key(
    mut param: Value,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {

    let (config, _digest) = config::drive::config()?;
    param["drive"] = crate::lookup_drive_name(&param, &config)?.into();

    if !tty::stdin_isatty() {
        bail!("no password input mechanism available");
    }

    let password = tty::read_password("Tepe Encryption Key Password: ")?;
    param["password"] = String::from_utf8(password)?.into();

    let info = &api2::tape::drive::API_METHOD_RESTORE_KEY;
    match info.handler {
        ApiHandler::Async(handler) => (handler)(param, info, rpcenv).await?,
        _ => unreachable!(),
    };

    Ok(())
}

#[api(
    input: {
        properties: {
            hint: {
                description: "Password restore hint.",
                type: String,
                min_length: 1,
                max_length: 32,
            },
        },
    },
)]
/// Create key (read password from stdin)
fn create_key(
    mut param: Value,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {

    if !tty::stdin_isatty() {
        bail!("no password input mechanism available");
    }

    let password = tty::read_and_verify_password("Tape Encryption Key Password: ")?;

    param["password"] = String::from_utf8(password)?.into();

    let info = &api2::config::tape_encryption_keys::API_METHOD_CREATE_KEY;
    let fingerprint = match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    println!("{}", fingerprint);

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
