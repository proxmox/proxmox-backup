/// Helper to run tape commands as root. Currently only required
/// to read and set the encryption key.
///
/// This command can use STDIN as tape device handle.
use std::fs::File;
use std::os::unix::io::{AsRawFd, FromRawFd};

use anyhow::{bail, Error};
use serde_json::Value;

use proxmox_router::{cli::*, RpcEnvironment};
use proxmox_schema::api;
use proxmox_uuid::Uuid;

use pbs_api_types::{
    Fingerprint, LtoTapeDrive, DRIVE_NAME_SCHEMA, LTO_DRIVE_PATH_SCHEMA, MEDIA_SET_UUID_SCHEMA,
    TAPE_ENCRYPTION_KEY_FINGERPRINT_SCHEMA,
};

use pbs_tape::linux_list_drives::{check_tape_is_lto_tape_device, open_lto_tape_device};

use proxmox_backup::tape::drive::{open_lto_tape_drive, LtoTapeHandle, TapeDriver};

fn get_tape_handle(param: &Value) -> Result<LtoTapeHandle, Error> {
    let handle = if let Some(name) = param["drive"].as_str() {
        let (config, _digest) = pbs_config::drive::config()?;
        let drive: LtoTapeDrive = config.lookup("lto", name)?;
        log::info!("using device {}", drive.path);
        open_lto_tape_drive(&drive)?
    } else if let Some(device) = param["device"].as_str() {
        log::info!("using device {}", device);
        LtoTapeHandle::new(open_lto_tape_device(device)?)?
    } else if let Some(true) = param["stdin"].as_bool() {
        log::info!("using stdin");
        let fd = std::io::stdin().as_raw_fd();
        let file = unsafe { File::from_raw_fd(fd) };
        check_tape_is_lto_tape_device(&file)?;
        LtoTapeHandle::new(file)?
    } else if let Ok(name) = std::env::var("PROXMOX_TAPE_DRIVE") {
        let (config, _digest) = pbs_config::drive::config()?;
        let drive: LtoTapeDrive = config.lookup("lto", &name)?;
        log::info!("using device {}", drive.path);
        open_lto_tape_drive(&drive)?
    } else {
        let (config, _digest) = pbs_config::drive::config()?;

        let mut drive_names = Vec::new();
        for (name, (section_type, _)) in config.sections.iter() {
            if section_type != "lto" {
                continue;
            }
            drive_names.push(name);
        }

        if drive_names.len() == 1 {
            let name = drive_names[0];
            let drive: LtoTapeDrive = config.lookup("lto", name)?;
            log::info!("using device {}", drive.path);
            open_lto_tape_drive(&drive)?
        } else {
            bail!("no drive/device specified");
        }
    };

    Ok(handle)
}

#[api(
    input: {
        properties: {
            fingerprint: {
                schema: TAPE_ENCRYPTION_KEY_FINGERPRINT_SCHEMA,
                optional: true,
            },
            uuid: {
                schema: MEDIA_SET_UUID_SCHEMA,
                optional: true,
            },
            drive: {
                schema: DRIVE_NAME_SCHEMA,
                optional: true,
            },
            device: {
                schema: LTO_DRIVE_PATH_SCHEMA,
                optional: true,
            },
            stdin: {
                description: "Use standard input as device handle.",
                type: bool,
                optional: true,
            },
        },
    },
)]
/// Set or clear encryption key
fn set_encryption(
    fingerprint: Option<Fingerprint>,
    uuid: Option<Uuid>,
    param: Value,
) -> Result<(), Error> {
    let result = proxmox_lang::try_block!({
        let mut handle = get_tape_handle(&param)?;

        match (fingerprint, uuid) {
            (Some(fingerprint), Some(uuid)) => {
                handle.set_encryption(Some((fingerprint, uuid)))?;
            }
            (Some(_), None) => {
                bail!("missing media set uuid");
            }
            (None, _) => {
                handle.set_encryption(None)?;
            }
        }

        Ok(())
    })
    .map_err(|err: Error| err.to_string());

    println!("{}", serde_json::to_string_pretty(&result)?);

    Ok(())
}

fn main() -> Result<(), Error> {
    init_cli_logger("PBS_LOG", "info");

    // check if we are user root or backup
    let backup_uid = pbs_config::backup_user()?.uid;
    let backup_gid = pbs_config::backup_group()?.gid;
    let running_uid = nix::unistd::Uid::current();
    let running_gid = nix::unistd::Gid::current();

    let effective_uid = nix::unistd::Uid::effective();
    if !effective_uid.is_root() {
        bail!("this program needs to be run with setuid root");
    }

    if !running_uid.is_root() && (running_uid != backup_uid || running_gid != backup_gid) {
        bail!(
            "Not running as backup user or group (got uid {} gid {})",
            running_uid,
            running_gid,
        );
    }

    let cmd_def =
        CliCommandMap::new().insert("encryption", CliCommand::new(&API_METHOD_SET_ENCRYPTION));

    let mut rpcenv = CliEnvironment::new();
    rpcenv.set_auth_id(Some(String::from("root@pam")));

    run_cli_command(cmd_def, rpcenv, None);

    Ok(())
}
