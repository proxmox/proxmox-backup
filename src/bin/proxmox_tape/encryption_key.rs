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
    tools::{
        self,
        paperkey::{
            PaperkeyFormat,
            generate_paper_key,
        },
    },
    config,
    api2::{
        self,
        types::{
            DRIVE_NAME_SCHEMA,
            TAPE_ENCRYPTION_KEY_FINGERPRINT_SCHEMA,
            PASSWORD_HINT_SCHEMA,
            Kdf,
        },
    },
    backup::Fingerprint,
    config::tape_encryption_keys::{
        load_key_configs,
        complete_key_fingerprint,
    },
};

pub fn encryption_key_commands() -> CommandLineInterface {

    let cmd_def = CliCommandMap::new()
        .insert("list", CliCommand::new(&API_METHOD_LIST_KEYS))
        .insert(
            "create",
            CliCommand::new(&API_METHOD_CREATE_KEY)
        )
        .insert(
            "change-passphrase",
            CliCommand::new(&API_METHOD_CHANGE_PASSPHRASE)
                .arg_param(&["fingerprint"])
                .completion_cb("fingerprint", complete_key_fingerprint)
        )
        .insert(
            "show",
            CliCommand::new(&API_METHOD_SHOW_KEY)
                .arg_param(&["fingerprint"])
                .completion_cb("fingerprint", complete_key_fingerprint)
        )
        .insert(
            "paperkey",
            CliCommand::new(&API_METHOD_PAPER_KEY)
                .arg_param(&["fingerprint"])
                .completion_cb("fingerprint", complete_key_fingerprint)
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
            fingerprint: {
                schema: TAPE_ENCRYPTION_KEY_FINGERPRINT_SCHEMA,
            },
            subject: {
                description: "Include the specified subject as title text.",
                optional: true,
            },
            "output-format": {
                type: PaperkeyFormat,
                optional: true,
            },
        },
    },
)]
/// Generate a printable, human readable text file containing the encryption key.
///
/// This also includes a scanable QR code for fast key restore.
fn paper_key(
    fingerprint: Fingerprint,
    subject: Option<String>,
    output_format: Option<PaperkeyFormat>,
) -> Result<(), Error> {

    let (config_map, _digest) = load_key_configs()?;

    let key_config = match config_map.get(&fingerprint) {
        Some(key_config) => key_config,
        None => bail!("tape encryption key '{}' does not exist.", fingerprint),
    };

    let data: String = serde_json::to_string_pretty(&key_config)?;

    generate_paper_key(std::io::stdout(), &data, subject, output_format)
}

#[api(
    input: {
        properties: {
            fingerprint: {
                schema: TAPE_ENCRYPTION_KEY_FINGERPRINT_SCHEMA,
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        },
    },
)]
/// Print the encryption key's metadata.
fn show_key(
    param: Value,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {

    let output_format = get_output_format(&param);

    let info = &api2::config::tape_encryption_keys::API_METHOD_READ_KEY;
    let mut data = match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    let options = proxmox::api::cli::default_table_format_options()
        .column(ColumnConfig::new("kdf"))
        .column(ColumnConfig::new("created").renderer(tools::format::render_epoch))
        .column(ColumnConfig::new("modified").renderer(tools::format::render_epoch))
        .column(ColumnConfig::new("fingerprint"))
        .column(ColumnConfig::new("hint"));

    format_and_print_result_full(&mut data, &info.returns, &output_format, &options);

    Ok(())
}

#[api(
    input: {
        properties: {
            kdf: {
                type: Kdf,
                optional: true,
            },
            fingerprint: {
                schema: TAPE_ENCRYPTION_KEY_FINGERPRINT_SCHEMA,
            },
            hint: {
                schema: PASSWORD_HINT_SCHEMA,
                optional: true,
            },
        },
    },
)]
/// Change the encryption key's password.
fn change_passphrase(
    mut param: Value,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {

    if !tty::stdin_isatty() {
        bail!("unable to change passphrase - no tty");
    }

    let password = tty::read_password("Current Tape Encryption Key Password: ")?;

    let new_password = tty::read_and_verify_password("New Tape Encryption Key Password: ")?;

    param["password"] = String::from_utf8(password)?.into();
    param["new-password"] = String::from_utf8(new_password)?.into();

    let info = &api2::config::tape_encryption_keys::API_METHOD_CHANGE_PASSPHRASE;
    match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    Ok(())
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
    param["drive"] = crate::extract_drive_name(&mut param, &config)?.into();

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
            kdf: {
                type: Kdf,
                optional: true,
            },
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
