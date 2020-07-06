use std::path::PathBuf;
use std::sync::Arc;
use std::os::unix::io::RawFd;
use std::path::Path;
use std::ffi::OsStr;

use anyhow::{bail, format_err, Error};
use serde_json::Value;
use tokio::signal::unix::{signal, SignalKind};
use nix::unistd::{fork, ForkResult, pipe};
use futures::select;
use futures::future::FutureExt;

use proxmox::{sortable, identity};
use proxmox::api::{ApiHandler, ApiMethod, RpcEnvironment, schema::*, cli::*};


use proxmox_backup::tools;
use proxmox_backup::backup::{
    load_and_decrypt_key,
    CryptConfig,
    IndexFile,
    BackupDir,
    BackupGroup,
    BufferedDynamicReader,
};

use proxmox_backup::client::*;

use crate::{
    REPO_URL_SCHEMA,
    extract_repository_from_value,
    complete_pxar_archive_name,
    complete_group_or_snapshot,
    complete_repository,
    record_repository,
    connect,
    api_datastore_latest_snapshot,
    BufferedDynamicReadAt,
};

#[sortable]
const API_METHOD_MOUNT: ApiMethod = ApiMethod::new(
    &ApiHandler::Sync(&mount),
    &ObjectSchema::new(
        "Mount pxar archive.",
        &sorted!([
            ("snapshot", false, &StringSchema::new("Group/Snapshot path.").schema()),
            ("archive-name", false, &StringSchema::new("Backup archive name.").schema()),
            ("target", false, &StringSchema::new("Target directory path.").schema()),
            ("repository", true, &REPO_URL_SCHEMA),
            ("keyfile", true, &StringSchema::new("Path to encryption key.").schema()),
            ("verbose", true, &BooleanSchema::new("Verbose output.").default(false).schema()),
        ]),
    )
);

pub fn mount_cmd_def() -> CliCommand {

    CliCommand::new(&API_METHOD_MOUNT)
        .arg_param(&["snapshot", "archive-name", "target"])
        .completion_cb("repository", complete_repository)
        .completion_cb("snapshot", complete_group_or_snapshot)
        .completion_cb("archive-name", complete_pxar_archive_name)
        .completion_cb("target", tools::complete_file_name)
}

fn mount(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let verbose = param["verbose"].as_bool().unwrap_or(false);
    if verbose {
        // This will stay in foreground with debug output enabled as None is
        // passed for the RawFd.
        return proxmox_backup::tools::runtime::main(mount_do(param, None));
    }

    // Process should be deamonized.
    // Make sure to fork before the async runtime is instantiated to avoid troubles.
    let pipe = pipe()?;
    match fork() {
        Ok(ForkResult::Parent { .. }) => {
            nix::unistd::close(pipe.1).unwrap();
            // Blocks the parent process until we are ready to go in the child
            let _res = nix::unistd::read(pipe.0, &mut [0]).unwrap();
            Ok(Value::Null)
        }
        Ok(ForkResult::Child) => {
            nix::unistd::close(pipe.0).unwrap();
            nix::unistd::setsid().unwrap();
            proxmox_backup::tools::runtime::main(mount_do(param, Some(pipe.1)))
        }
        Err(_) => bail!("failed to daemonize process"),
    }
}

async fn mount_do(param: Value, pipe: Option<RawFd>) -> Result<Value, Error> {
    let repo = extract_repository_from_value(&param)?;
    let archive_name = tools::required_string_param(&param, "archive-name")?;
    let target = tools::required_string_param(&param, "target")?;
    let client = connect(repo.host(), repo.user())?;

    record_repository(&repo);

    let path = tools::required_string_param(&param, "snapshot")?;
    let (backup_type, backup_id, backup_time) = if path.matches('/').count() == 1 {
        let group: BackupGroup = path.parse()?;
        api_datastore_latest_snapshot(&client, repo.store(), group).await?
    } else {
        let snapshot: BackupDir = path.parse()?;
        (snapshot.group().backup_type().to_owned(), snapshot.group().backup_id().to_owned(), snapshot.backup_time())
    };

    let keyfile = param["keyfile"].as_str().map(PathBuf::from);
    let crypt_config = match keyfile {
        None => None,
        Some(path) => {
            let (key, _) = load_and_decrypt_key(&path, &crate::key::get_encryption_key_password)?;
            Some(Arc::new(CryptConfig::new(key)?))
        }
    };

    let server_archive_name = if archive_name.ends_with(".pxar") {
        format!("{}.didx", archive_name)
    } else {
        bail!("Can only mount pxar archives.");
    };

    let client = BackupReader::start(
        client,
        crypt_config.clone(),
        repo.store(),
        &backup_type,
        &backup_id,
        backup_time,
        true,
    ).await?;

    let manifest = client.download_manifest().await?;

    if server_archive_name.ends_with(".didx") {
        let index = client.download_dynamic_index(&manifest, &server_archive_name).await?;
        let most_used = index.find_most_used_chunks(8);
        let chunk_reader = RemoteChunkReader::new(client.clone(), crypt_config, most_used);
        let reader = BufferedDynamicReader::new(index, chunk_reader);
        let archive_size = reader.archive_size();
        let reader: proxmox_backup::pxar::fuse::Reader =
            Arc::new(BufferedDynamicReadAt::new(reader));
        let decoder = proxmox_backup::pxar::fuse::Accessor::new(reader, archive_size).await?;
        let options = OsStr::new("ro,default_permissions");

        let session = proxmox_backup::pxar::fuse::Session::mount(
            decoder,
            &options,
            false,
            Path::new(target),
        )
        .map_err(|err| format_err!("pxar mount failed: {}", err))?;

        if let Some(pipe) = pipe {
            nix::unistd::chdir(Path::new("/")).unwrap();
            // Finish creation of daemon by redirecting filedescriptors.
            let nullfd = nix::fcntl::open(
                "/dev/null",
                nix::fcntl::OFlag::O_RDWR,
                nix::sys::stat::Mode::empty(),
            ).unwrap();
            nix::unistd::dup2(nullfd, 0).unwrap();
            nix::unistd::dup2(nullfd, 1).unwrap();
            nix::unistd::dup2(nullfd, 2).unwrap();
            if nullfd > 2 {
                nix::unistd::close(nullfd).unwrap();
            }
            // Signal the parent process that we are done with the setup and it can
            // terminate.
            nix::unistd::write(pipe, &[0u8])?;
            nix::unistd::close(pipe).unwrap();
        }

        let mut interrupt = signal(SignalKind::interrupt())?;
        select! {
            res = session.fuse() => res?,
            _ = interrupt.recv().fuse() => {
                // exit on interrupted
            }
        }
    } else {
        bail!("unknown archive file extension (expected .pxar)");
    }

    Ok(Value::Null)
}
