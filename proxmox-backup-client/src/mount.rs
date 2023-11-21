use std::collections::HashMap;
use std::ffi::OsStr;
use std::hash::BuildHasher;
use std::os::unix::io::{AsRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{bail, format_err, Error};
use futures::future::FutureExt;
use futures::select;
use futures::stream::{StreamExt, TryStreamExt};
use nix::unistd::{fork, ForkResult};
use serde_json::Value;
use tokio::signal::unix::{signal, SignalKind};

use proxmox_router::{cli::*, ApiHandler, ApiMethod, RpcEnvironment};
use proxmox_schema::*;
use proxmox_sortable_macro::sortable;

use pbs_api_types::BackupNamespace;
use pbs_client::tools::key_source::get_encryption_key_password;
use pbs_client::{BackupReader, RemoteChunkReader};
use pbs_datastore::cached_chunk_reader::CachedChunkReader;
use pbs_datastore::dynamic_index::BufferedDynamicReader;
use pbs_datastore::index::IndexFile;
use pbs_key_config::load_and_decrypt_key;
use pbs_tools::crypt_config::CryptConfig;
use pbs_tools::json::required_string_param;

use crate::{
    complete_group_or_snapshot, complete_img_archive_name, complete_namespace,
    complete_pxar_archive_name, complete_repository, connect, dir_or_last_from_group,
    extract_repository_from_value, optional_ns_param, record_repository, BufferedDynamicReadAt,
    REPO_URL_SCHEMA,
};

#[sortable]
const API_METHOD_MOUNT: ApiMethod = ApiMethod::new(
    &ApiHandler::Sync(&mount),
    &ObjectSchema::new(
        "Mount pxar archive.",
        &sorted!([
            ("ns", true, &BackupNamespace::API_SCHEMA,),
            (
                "snapshot",
                false,
                &StringSchema::new("Group/Snapshot path.").schema()
            ),
            (
                "archive-name",
                false,
                &StringSchema::new("Backup archive name.").schema()
            ),
            (
                "target",
                false,
                &StringSchema::new("Target directory path.").schema()
            ),
            ("repository", true, &REPO_URL_SCHEMA),
            (
                "keyfile",
                true,
                &StringSchema::new("Path to encryption key.").schema()
            ),
            (
                "verbose",
                true,
                &BooleanSchema::new("Verbose output and stay in foreground.")
                    .default(false)
                    .schema()
            ),
        ]),
    ),
);

#[sortable]
const API_METHOD_MAP: ApiMethod = ApiMethod::new(
    &ApiHandler::Sync(&mount),
    &ObjectSchema::new(
        "Map a drive image from a VM backup to a local loopback device. Use 'unmap' to undo.
WARNING: Only do this with *trusted* backups!",
        &sorted!([
            ("ns", true, &BackupNamespace::API_SCHEMA,),
            (
                "snapshot",
                false,
                &StringSchema::new("Group/Snapshot path.").schema()
            ),
            (
                "archive-name",
                false,
                &StringSchema::new("Backup archive name.").schema()
            ),
            ("repository", true, &REPO_URL_SCHEMA),
            (
                "keyfile",
                true,
                &StringSchema::new("Path to encryption key.").schema()
            ),
            (
                "verbose",
                true,
                &BooleanSchema::new("Verbose output and stay in foreground.")
                    .default(false)
                    .schema()
            ),
        ]),
    ),
);

#[sortable]
const API_METHOD_UNMAP: ApiMethod = ApiMethod::new(
    &ApiHandler::Sync(&unmap),
    &ObjectSchema::new(
        "Unmap a loop device mapped with 'map' and release all resources.",
        &sorted!([(
            "name",
            true,
            &StringSchema::new(concat!(
                "Archive name, path to loopdev (/dev/loopX) or loop device number. ",
                "Omit to list all current mappings and force cleaning up leftover instances."
            ))
            .schema()
        ),]),
    ),
);

pub fn mount_cmd_def() -> CliCommand {
    CliCommand::new(&API_METHOD_MOUNT)
        .arg_param(&["snapshot", "archive-name", "target"])
        .completion_cb("repository", complete_repository)
        .completion_cb("ns", complete_namespace)
        .completion_cb("snapshot", complete_group_or_snapshot)
        .completion_cb("archive-name", complete_pxar_archive_name)
        .completion_cb("target", complete_file_name)
}

pub fn map_cmd_def() -> CliCommand {
    CliCommand::new(&API_METHOD_MAP)
        .arg_param(&["snapshot", "archive-name"])
        .completion_cb("repository", complete_repository)
        .completion_cb("ns", complete_namespace)
        .completion_cb("snapshot", complete_group_or_snapshot)
        .completion_cb("archive-name", complete_img_archive_name)
}

pub fn unmap_cmd_def() -> CliCommand {
    CliCommand::new(&API_METHOD_UNMAP)
        .arg_param(&["name"])
        .completion_cb("name", complete_mapping_names)
}

fn complete_mapping_names<S: BuildHasher>(
    _arg: &str,
    _param: &HashMap<String, String, S>,
) -> Vec<String> {
    match pbs_fuse_loop::find_all_mappings() {
        Ok(mappings) => mappings
            .filter_map(|(name, _)| proxmox_sys::systemd::unescape_unit(&name).ok())
            .collect(),
        Err(_) => Vec::new(),
    }
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
        return proxmox_async::runtime::main(mount_do(param, None));
    }

    // Process should be daemonized.
    // Make sure to fork before the async runtime is instantiated to avoid troubles.
    let (pr, pw) = proxmox_sys::pipe()?;
    let pr: OwnedFd = pr.into(); // until next sys bump
    let pw: OwnedFd = pw.into();
    match unsafe { fork() } {
        Ok(ForkResult::Parent { .. }) => {
            drop(pw);
            // Blocks the parent process until we are ready to go in the child
            let _res = nix::unistd::read(pr.as_raw_fd(), &mut [0]).unwrap();
            Ok(Value::Null)
        }
        Ok(ForkResult::Child) => {
            drop(pr);
            nix::unistd::setsid().unwrap();
            proxmox_async::runtime::main(mount_do(param, Some(pw)))
        }
        Err(_) => bail!("failed to daemonize process"),
    }
}

async fn mount_do(param: Value, pipe: Option<OwnedFd>) -> Result<Value, Error> {
    let repo = extract_repository_from_value(&param)?;
    let archive_name = required_string_param(&param, "archive-name")?;
    let client = connect(&repo)?;

    let target = param["target"].as_str();

    record_repository(&repo);

    let backup_ns = optional_ns_param(&param)?;
    let path = required_string_param(&param, "snapshot")?;
    let backup_dir = dir_or_last_from_group(&client, &repo, &backup_ns, path).await?;

    let keyfile = param["keyfile"].as_str().map(PathBuf::from);
    let crypt_config = match keyfile {
        None => None,
        Some(path) => {
            log::info!("Encryption key file: '{:?}'", path);
            let (key, _, fingerprint) = load_and_decrypt_key(&path, &get_encryption_key_password)?;
            log::info!("Encryption key fingerprint: '{}'", fingerprint);
            Some(Arc::new(CryptConfig::new(key)?))
        }
    };

    let server_archive_name = if archive_name.ends_with(".pxar") {
        if target.is_none() {
            bail!("use the 'mount' command to mount pxar archives");
        }
        format!("{}.didx", archive_name)
    } else if archive_name.ends_with(".img") {
        if target.is_some() {
            bail!("use the 'map' command to map drive images");
        }
        format!("{}.fidx", archive_name)
    } else {
        bail!("Can only mount/map pxar archives and drive images.");
    };

    let client = BackupReader::start(
        &client,
        crypt_config.clone(),
        repo.store(),
        &backup_ns,
        &backup_dir,
        true,
    )
    .await?;

    let (manifest, _) = client.download_manifest().await?;
    manifest.check_fingerprint(crypt_config.as_ref().map(Arc::as_ref))?;

    let file_info = manifest.lookup_file_info(&server_archive_name)?;

    let daemonize = || -> Result<(), Error> {
        if let Some(pipe) = pipe {
            nix::unistd::chdir(Path::new("/")).unwrap();
            // Finish creation of daemon by redirecting filedescriptors.
            let nullfd = nix::fcntl::open(
                "/dev/null",
                nix::fcntl::OFlag::O_RDWR,
                nix::sys::stat::Mode::empty(),
            )
            .unwrap();
            nix::unistd::dup2(nullfd, 0).unwrap();
            nix::unistd::dup2(nullfd, 1).unwrap();
            nix::unistd::dup2(nullfd, 2).unwrap();
            if nullfd > 2 {
                nix::unistd::close(nullfd).unwrap();
            }
            // Signal the parent process that we are done with the setup and it can
            // terminate.
            nix::unistd::write(pipe.as_raw_fd(), &[0u8])?;
            let _: OwnedFd = pipe;
        }

        Ok(())
    };

    let options = OsStr::new("ro,default_permissions");

    // handle SIGINT and SIGTERM
    let mut interrupt_int = signal(SignalKind::interrupt())?;
    let mut interrupt_term = signal(SignalKind::terminate())?;

    let mut interrupt =
        futures::future::select(interrupt_int.recv().boxed(), interrupt_term.recv().boxed());

    if server_archive_name.ends_with(".didx") {
        let index = client
            .download_dynamic_index(&manifest, &server_archive_name)
            .await?;
        let most_used = index.find_most_used_chunks(8);
        let chunk_reader = RemoteChunkReader::new(
            client.clone(),
            crypt_config,
            file_info.chunk_crypt_mode(),
            most_used,
        );
        let reader = BufferedDynamicReader::new(index, chunk_reader);
        let archive_size = reader.archive_size();
        let reader: pbs_pxar_fuse::Reader = Arc::new(BufferedDynamicReadAt::new(reader));
        let decoder = pbs_pxar_fuse::Accessor::new(reader, archive_size).await?;

        let session =
            pbs_pxar_fuse::Session::mount(decoder, options, false, Path::new(target.unwrap()))
                .map_err(|err| format_err!("pxar mount failed: {}", err))?;

        daemonize()?;

        select! {
            res = session.fuse() => res?,
            _ = interrupt => {
                // exit on interrupted
            }
        }
    } else if server_archive_name.ends_with(".fidx") {
        let index = client
            .download_fixed_index(&manifest, &server_archive_name)
            .await?;
        let size = index.index_bytes();
        let chunk_reader = RemoteChunkReader::new(
            client.clone(),
            crypt_config,
            file_info.chunk_crypt_mode(),
            HashMap::new(),
        );
        let reader = CachedChunkReader::new(chunk_reader, index, 8).seekable();

        let name = &format!("{}:{}/{}", repo, path, archive_name);
        let name_escaped = proxmox_sys::systemd::escape_unit(name, false);

        let mut session =
            pbs_fuse_loop::FuseLoopSession::map_loop(size, reader, &name_escaped, options).await?;
        let loopdev = session.loopdev_path.clone();

        let (st_send, st_recv) = futures::channel::mpsc::channel(1);
        let (mut abort_send, abort_recv) = futures::channel::mpsc::channel(1);
        let mut st_recv = st_recv.fuse();
        let mut session_fut = session.main(st_send, abort_recv).boxed().fuse();

        // poll until loop file is mapped (or errors)
        select! {
            _res = session_fut => {
                bail!("FUSE session unexpectedly ended before loop file mapping");
            },
            res = st_recv.try_next() => {
                if let Err(err) = res {
                    // init went wrong, abort now
                    abort_send.try_send(()).map_err(|err|
                        format_err!("error while sending abort signal - {}", err))?;
                    // ignore and keep original error cause
                    let _ = session_fut.await;
                    return Err(err);
                }
            }
        }

        // daemonize only now to be able to print mapped loopdev or startup errors
        log::info!("Image '{}' mapped on {}", name, loopdev);
        daemonize()?;

        // continue polling until complete or interrupted (which also happens on unmap)
        select! {
            res = session_fut => res?,
            _ = interrupt => {
                // exit on interrupted
                abort_send.try_send(()).map_err(|err|
                    format_err!("error while sending abort signal - {}", err))?;
                session_fut.await?;
            }
        }

        log::info!("Image unmapped");
    } else {
        bail!("unknown archive file extension (expected .pxar or .img)");
    }

    Ok(Value::Null)
}

fn unmap(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {
    let mut name = match param["name"].as_str() {
        Some(name) => name.to_owned(),
        None => {
            pbs_fuse_loop::cleanup_unused_run_files(None);
            let mut any = false;
            for (backing, loopdev) in pbs_fuse_loop::find_all_mappings()? {
                let name = proxmox_sys::systemd::unescape_unit(&backing)?;
                log::info!(
                    "{}:\t{}",
                    loopdev.unwrap_or_else(|| "(unmapped)".to_string()),
                    name
                );
                any = true;
            }
            if !any {
                log::info!("Nothing mapped.");
            }
            return Ok(Value::Null);
        }
    };

    // allow loop device number alone
    if let Ok(num) = name.parse::<u8>() {
        name = format!("/dev/loop{}", num);
    }

    if name.starts_with("/dev/loop") {
        pbs_fuse_loop::unmap_loopdev(name)?;
    } else {
        let name = proxmox_sys::systemd::escape_unit(&name, false);
        pbs_fuse_loop::unmap_name(name)?;
    }

    Ok(Value::Null)
}
