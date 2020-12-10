use anyhow::{format_err, Error};
use serde_json::Value;

use proxmox::{
    api::{
        api,
        cli::*,
        ApiHandler,
        RpcEnvironment,
        section_config::SectionConfigData,
    },
};

use proxmox_backup::{
    api2::{
        self,
        types::{
            DRIVE_ID_SCHEMA,
            MEDIA_LABEL_SCHEMA,
        },
    },
    config::{
        self,
        drive::complete_drive_name,
    },
    tape::{
        complete_media_changer_id,
    },
};

mod proxmox_tape;
use proxmox_tape::*;

fn lookup_drive_name(
    param: &Value,
    config: &SectionConfigData,
) -> Result<String, Error> {

    let drive = param["drive"]
        .as_str()
        .map(String::from)
        .or_else(|| std::env::var("PROXMOX_TAPE_DRIVE").ok())
        .or_else(||  {

            let mut drive_names = Vec::new();

            for (name, (section_type, _)) in config.sections.iter() {

                if !(section_type == "linux" || section_type == "virtual") { continue; }
                drive_names.push(name);
            }

            if drive_names.len() == 1 {
                Some(drive_names[0].to_owned())
            } else {
                None
            }
        })
        .ok_or_else(|| format_err!("unable to get (default) drive name"))?;

    Ok(drive)
}

#[api(
    input: {
        properties: {
            drive: {
                schema: DRIVE_ID_SCHEMA,
                optional: true,
            },
            fast: {
                description: "Use fast erase.",
                type: bool,
                optional: true,
                default: true,
            },
        },
    },
)]
/// Erase media
fn erase_media(
    mut param: Value,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {

    let (config, _digest) = config::drive::config()?;

    param["drive"] = lookup_drive_name(&param, &config)?.into();

    let info = &api2::tape::drive::API_METHOD_ERASE_MEDIA;

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
                schema: DRIVE_ID_SCHEMA,
                optional: true,
            },
        },
    },
)]
/// Rewind tape
fn rewind(
    mut param: Value,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {

    let (config, _digest) = config::drive::config()?;

    param["drive"] = lookup_drive_name(&param, &config)?.into();

    let info = &api2::tape::drive::API_METHOD_REWIND;

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
                schema: DRIVE_ID_SCHEMA,
                optional: true,
            },
        },
    },
)]
/// Eject/Unload drive media
fn eject_media(
    mut param: Value,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {

    let (config, _digest) = config::drive::config()?;

    param["drive"] = lookup_drive_name(&param, &config)?.into();

    let info = &api2::tape::drive::API_METHOD_EJECT_MEDIA;

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
                schema: DRIVE_ID_SCHEMA,
                optional: true,
            },
            "changer-id": {
                schema: MEDIA_LABEL_SCHEMA,
            },
        },
    },
)]
/// Load media
fn load_media(
    mut param: Value,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {

    let (config, _digest) = config::drive::config()?;

    param["drive"] = lookup_drive_name(&param, &config)?.into();

    let info = &api2::tape::drive::API_METHOD_LOAD_MEDIA;

    match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    Ok(())
}

fn main() {

    let cmd_def = CliCommandMap::new()
        .insert(
            "rewind",
            CliCommand::new(&API_METHOD_REWIND)
                .completion_cb("drive", complete_drive_name)
        )
        .insert(
            "erase",
            CliCommand::new(&API_METHOD_ERASE_MEDIA)
                .completion_cb("drive", complete_drive_name)
        )
        .insert(
            "eject",
            CliCommand::new(&API_METHOD_EJECT_MEDIA)
                .completion_cb("drive", complete_drive_name)
        )
        .insert("changer", changer_commands())
        .insert("drive", drive_commands())
        .insert("pool", pool_commands())
        .insert(
            "load-media",
            CliCommand::new(&API_METHOD_LOAD_MEDIA)
                .arg_param(&["changer-id"])
                .completion_cb("drive", complete_drive_name)
                .completion_cb("changer-id", complete_media_changer_id)
        )
        ;

    let mut rpcenv = CliEnvironment::new();
    rpcenv.set_auth_id(Some(String::from("root@pam")));

    proxmox_backup::tools::runtime::main(run_async_cli_command(cmd_def, rpcenv));
}
