use anyhow::{bail, format_err, Error};
use serde_json::Value;

use proxmox_router::{cli::*, ApiHandler, RpcEnvironment};
use proxmox_schema::{api, param_bail};
use proxmox_sys::linux::tty;

use pbs_api_types::{
    Fingerprint, Kdf, DRIVE_NAME_SCHEMA, TAPE_ENCRYPTION_KEY_FINGERPRINT_SCHEMA,
    PASSWORD_HINT_SCHEMA,
};

use pbs_datastore::paperkey::{PaperkeyFormat, generate_paper_key};
use pbs_config::tape_encryption_keys::{load_key_configs,complete_key_fingerprint};
use pbs_config::key_config::KeyConfig;

use proxmox_backup::api2;

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

    let options = proxmox_router::cli::default_table_format_options()
        .column(ColumnConfig::new("kdf"))
        .column(ColumnConfig::new("created").renderer(pbs_tools::format::render_epoch))
        .column(ColumnConfig::new("modified").renderer(pbs_tools::format::render_epoch))
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
            },
            force: {
                optional: true,
                type: bool,
                description: "Reset the passphrase for a tape key, without asking for the old one.",
                default: false,
            },
        },
    },
)]
/// Change the encryption key's password.
fn change_passphrase(
    force: bool,
    mut param: Value,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {

    if !tty::stdin_isatty() {
        bail!("unable to change passphrase - no tty");
    }

    if force {
        param["force"] = serde_json::Value::Bool(true);
    } else {
        let password = tty::read_password("Current Tape Encryption Key Password: ")?;
        param["password"] = String::from_utf8(password)?.into();
    }

    let new_password = tty::read_and_verify_password("New Tape Encryption Key Password: ")?;

    param["new-password"] = String::from_utf8(new_password)?.into();

    let info = &api2::config::tape_encryption_keys::API_METHOD_CHANGE_PASSPHRASE;
    match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    Ok(())
}

pub const BEGIN_MARKER: &str = "-----BEGIN PROXMOX BACKUP KEY-----";
pub const END_MARKER: &str = "-----END PROXMOX BACKUP KEY-----";

#[api(
    input: {
        properties: {
            drive: {
                schema: DRIVE_NAME_SCHEMA,
                optional: true,
            },
            "backupkey": {
                description: "Importing a previously exported backupkey with either an exported paperkey-file, json-string or a json-file",
                type: String,
                optional: true,
            },
        },
    },
)]
/// Restore encryption key from tape (read password from stdin)
async fn restore_key(
    mut param: Value,
    backupkey: Option<String>,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {

    let (config, _digest) = pbs_config::drive::config()?;

    let drive = crate::extract_drive_name(&mut param, &config);

    if !tty::stdin_isatty() {
        bail!("no password input mechanism available");
    }

    if (drive.is_err() && backupkey.is_none()) || (drive.is_ok() && backupkey.is_some()) {
        param_bail!(
            "drive",
            format_err!("Please specify either a valid drive name or a backupkey")
        );
    }

    if drive.is_ok() {
        param["drive"] = drive.unwrap().into();
    }

    if let Some(backupkey) = backupkey {
        if serde_json::from_str::<KeyConfig>(&backupkey).is_ok() {
            // json as Parameter
            println!("backupkey to import: {}", backupkey);
            param["backupkey"] = backupkey.into();
        } else {
            println!("backupkey is not a valid json. Interpreting Parameter as a filename");
            let data = proxmox_sys::fs::file_read_string(backupkey)?;
            if serde_json::from_str::<KeyConfig>(&data).is_ok() {
                // standalone json-file
                println!("backupkey to import: {}", data);
                param["backupkey"] = data.into();
            } else {
                // exported paperkey-file
                let start = data
                    .find(BEGIN_MARKER)
                    .ok_or_else(|| format_err!("cannot find key start marker"))?
                    + BEGIN_MARKER.len();
                let data_remain = &data[start..];
                let end = data_remain
                    .find(END_MARKER)
                    .ok_or_else(|| format_err!("cannot find key end marker below start marker"))?;
                let backupkey_extract = &data_remain[..end];
                println!("backupkey to import: {}", backupkey_extract);
                param["backupkey"] = backupkey_extract.into();
            }
        }
    }

    let password = tty::read_password("Tape Encryption Key Password: ")?;
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
