///! File-restore API running inside the restore VM
use std::ffi::OsStr;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use anyhow::{bail, Error};
use futures::FutureExt;
use hyper::http::request::Parts;
use hyper::{header, Body, Response, StatusCode};
use log::error;
use serde_json::Value;
use tokio::sync::Semaphore;

use pathpatterns::{MatchEntry, MatchPattern, MatchType, Pattern};
use proxmox_compression::{tar::tar_directory, zip::zip_directory, zstd::ZstdEncoder};
use proxmox_router::{
    list_subdirs_api_method, ApiHandler, ApiMethod, ApiResponseFuture, Permission, Router,
    RpcEnvironment, SubdirMap,
};
use proxmox_schema::*;
use proxmox_sortable_macro::sortable;
use proxmox_sys::fs::read_subdir;

use pbs_api_types::file_restore::{FileRestoreFormat, RestoreDaemonStatus};
use pbs_client::pxar::{create_archive, Flags, PxarCreateOptions, ENCODER_MAX_ENTRIES};
use pbs_datastore::catalog::{ArchiveEntry, DirEntryAttribute};
use pbs_tools::json::required_string_param;

use pxar::encoder::aio::TokioWriter;

use super::{disk::ResolveResult, watchdog_inhibit, watchdog_ping, watchdog_remaining};

// NOTE: All API endpoints must have Permission::Superuser, as the configs for authentication do
// not exist within the restore VM. Safety is guaranteed by checking a ticket via a custom ApiAuth.

const SUBDIRS: SubdirMap = &[
    ("extract", &Router::new().get(&API_METHOD_EXTRACT)),
    ("list", &Router::new().get(&API_METHOD_LIST)),
    ("status", &Router::new().get(&API_METHOD_STATUS)),
    ("stop", &Router::new().get(&API_METHOD_STOP)),
];

pub const ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(SUBDIRS))
    .subdirs(SUBDIRS);

static DOWNLOAD_SEM: Semaphore = Semaphore::const_new(8);

fn read_uptime() -> Result<f32, Error> {
    let uptime = fs::read_to_string("/proc/uptime")?;
    // unwrap the Option, if /proc/uptime is empty we have bigger problems
    Ok(uptime.split_ascii_whitespace().next().unwrap().parse()?)
}

#[api(
    input: {
        properties: {
            "keep-timeout": {
                type: bool,
                description: "If true, do not reset the watchdog timer on this API call.",
                default: false,
                optional: true,
            },
        },
    },
    access: {
        description: "Permissions are handled outside restore VM. This call can be made without a ticket, but keep-timeout is always assumed 'true' then.",
        permission: &Permission::World,
    },
    returns: {
        type: RestoreDaemonStatus,
    }
)]
/// General status information
fn status(
    rpcenv: &mut dyn RpcEnvironment,
    keep_timeout: bool,
) -> Result<RestoreDaemonStatus, Error> {
    if !keep_timeout && rpcenv.get_auth_id().is_some() {
        watchdog_ping();
    }
    Ok(RestoreDaemonStatus {
        uptime: read_uptime()? as i64,
        timeout: watchdog_remaining(),
    })
}

#[api(
    access: {
        description: "Permissions are handled outside restore VM.",
        permission: &Permission::Superuser,
    },
)]
/// Stop the restore VM immediately, this will never return if successful
fn stop() {
    use nix::sys::reboot;
    println!("/stop called, shutting down");
    let err = reboot::reboot(reboot::RebootMode::RB_POWER_OFF).unwrap_err();
    println!("'reboot' syscall failed: {}", err);
    std::process::exit(1);
}

fn get_dir_entry(path: &Path) -> Result<DirEntryAttribute, Error> {
    use nix::sys::stat;

    let stat = stat::stat(path)?;
    Ok(match stat.st_mode & libc::S_IFMT {
        libc::S_IFREG => DirEntryAttribute::File {
            size: stat.st_size as u64,
            mtime: stat.st_mtime,
        },
        libc::S_IFDIR => DirEntryAttribute::Directory { start: 0 },
        _ => bail!("unsupported file type: {}", stat.st_mode),
    })
}

#[api(
    streaming: true,
    input: {
        properties: {
            "path": {
                type: String,
                description: "base64-encoded path to list files and directories under",
            },
        },
    },
    access: {
        description: "Permissions are handled outside restore VM.",
        permission: &Permission::Superuser,
    },
)]
/// List file details for given file or a list of files and directories under the given path if it
/// points to a directory.
fn list(
    path: String,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<ArchiveEntry>, Error> {
    watchdog_ping();

    let mut res = Vec::new();

    let param_path = base64::decode(path)?;
    let mut path = param_path.clone();
    if let Some(b'/') = path.last() {
        path.pop();
    }
    let path_str = OsStr::from_bytes(&path[..]);
    let param_path_buf = Path::new(path_str);

    let query_result = proxmox_async::runtime::block_in_place(move || {
        let mut disk_state = crate::DISK_STATE.lock().unwrap();
        disk_state.resolve(param_path_buf)
    })?;

    match query_result {
        ResolveResult::Path(vm_path) => {
            let root_entry = get_dir_entry(&vm_path)?;
            match root_entry {
                DirEntryAttribute::File { .. } => {
                    // list on file, return details
                    res.push(ArchiveEntry::new(&param_path, Some(&root_entry)));
                }
                DirEntryAttribute::Directory { .. } => {
                    // list on directory, return all contained files/dirs
                    for f in read_subdir(libc::AT_FDCWD, &vm_path)?.flatten() {
                        let name = f.file_name().to_bytes();
                        let path = &Path::new(OsStr::from_bytes(name));
                        if path.components().count() == 1 {
                            // ignore '.' and '..'
                            match path.components().next().unwrap() {
                                std::path::Component::CurDir | std::path::Component::ParentDir => {
                                    continue
                                }
                                _ => {}
                            }
                        }
                        let mut full_vm_path = PathBuf::new();
                        full_vm_path.push(&vm_path);
                        full_vm_path.push(path);
                        let mut full_path = PathBuf::new();
                        full_path.push(param_path_buf);
                        full_path.push(path);
                        let entry = get_dir_entry(&full_vm_path);
                        if let Ok(entry) = entry {
                            res.push(ArchiveEntry::new(
                                full_path.as_os_str().as_bytes(),
                                Some(&entry),
                            ));
                        }
                    }
                }
                _ => unreachable!(),
            }
        }
        ResolveResult::BucketTypes(types) => {
            for t in types {
                let mut t_path = path.clone();
                t_path.push(b'/');
                t_path.extend(t.as_bytes());
                res.push(ArchiveEntry::new(&t_path[..], None));
            }
        }
        ResolveResult::BucketComponents(comps) => {
            for c in comps {
                let mut c_path = path.clone();
                c_path.push(b'/');
                c_path.extend(c.0.as_bytes());
                res.push(ArchiveEntry::new_with_size(
                    &c_path[..],
                    // this marks the beginning of a filesystem, i.e. '/', so this is a Directory
                    Some(&DirEntryAttribute::Directory { start: 0 }),
                    c.1,
                ));
            }
        }
    }

    Ok(res)
}

#[sortable]
pub const API_METHOD_EXTRACT: ApiMethod = ApiMethod::new(
    &ApiHandler::AsyncHttp(&extract),
    &ObjectSchema::new(
        "Extract a file or directory from the VM as a pxar archive.",
        &sorted!([
            (
                "path",
                false,
                &StringSchema::new("base64-encoded path to list files and directories under")
                    .schema()
            ),
            (
                "pxar",
                true,
                &BooleanSchema::new(concat!(
                    "if true, return a pxar archive, otherwise either the ",
                    "file content or the directory as a zip file. DEPRECATED: use 'format' instead."
                ))
                .default(true)
                .schema()
            ),
            ("format", true, &FileRestoreFormat::API_SCHEMA,),
            (
                "zstd",
                true,
                &BooleanSchema::new(concat!("if true, zstd compresses the result.",))
                    .default(false)
                    .schema()
            ),
        ]),
    ),
)
.access(None, &Permission::Superuser);

fn extract(
    _parts: Parts,
    _req_body: Body,
    param: Value,
    _info: &ApiMethod,
    _rpcenv: Box<dyn RpcEnvironment>,
) -> ApiResponseFuture {
    // download can take longer than watchdog timeout, inhibit until done
    let _inhibitor = watchdog_inhibit();
    async move {
        let _inhibitor = _inhibitor;

        let _permit = match DOWNLOAD_SEM.try_acquire() {
            Ok(permit) => permit,
            Err(_) => bail!("maximum concurrent download limit reached, please wait for another restore to finish before attempting a new one"),
        };

        let path = required_string_param(&param, "path")?;
        let mut path = base64::decode(path)?;
        if let Some(b'/') = path.last() {
            path.pop();
        }
        let path = Path::new(OsStr::from_bytes(&path[..]));

        let format = match (param["format"].as_str(), param["pxar"].as_bool()) {
            (Some(format), None) => format.to_string(),
            (Some(_), Some(_)) => bail!("cannot set 'pxar' and 'format' simultaneously"),
            // FIXME, pxar 'false' defaulted to either zip or plain, remove with 3.0
            (None, Some(false) | None) => String::new(),
            (None, Some(true)) => "pxar".to_string(),
        };

        let query_result = proxmox_async::runtime::block_in_place(move || {
            let mut disk_state = crate::DISK_STATE.lock().unwrap();
            disk_state.resolve(path)
        })?;

        let vm_path = match query_result {
            ResolveResult::Path(vm_path) => vm_path,
            _ => bail!("invalid path, cannot restore meta-directory: {:?}", path),
        };

        // check here so we can return a real error message, failing in the async task will stop
        // the transfer, but not return a useful message
        if !vm_path.exists() {
            bail!("file or directory {:?} does not exist", path);
        }

        let (mut writer, reader) = tokio::io::duplex(1024 * 64);

        if format == "pxar" {
            tokio::spawn(async move {
                let _inhibitor = _inhibitor;
                let _permit = _permit;
                let result = async move {
                    // pxar always expects a directory as it's root, so to accommodate files as
                    // well we encode the parent dir with a filter only matching the target instead
                    let mut patterns = vec![MatchEntry::new(
                        MatchPattern::Pattern(Pattern::path(b"*").unwrap()),
                        MatchType::Exclude,
                    )];

                    let name = match vm_path.file_name() {
                        Some(name) => name,
                        None => bail!("no file name found for path: {:?}", vm_path),
                    };

                    if vm_path.is_dir() {
                        let mut pat = name.as_bytes().to_vec();
                        patterns.push(MatchEntry::new(
                            MatchPattern::Pattern(Pattern::path(pat.clone())?),
                            MatchType::Include,
                        ));
                        pat.extend(b"/**/*".iter());
                        patterns.push(MatchEntry::new(
                            MatchPattern::Pattern(Pattern::path(pat)?),
                            MatchType::Include,
                        ));
                    } else {
                        patterns.push(MatchEntry::new(
                            MatchPattern::Literal(name.as_bytes().to_vec()),
                            MatchType::Include,
                        ));
                    }

                    let dir_path = vm_path.parent().unwrap_or_else(|| Path::new("/"));
                    let dir = nix::dir::Dir::open(
                        dir_path,
                        nix::fcntl::OFlag::O_NOFOLLOW,
                        nix::sys::stat::Mode::empty(),
                    )?;

                    let options = PxarCreateOptions {
                        entries_max: ENCODER_MAX_ENTRIES,
                        device_set: None,
                        patterns,
                        skip_lost_and_found: false,
                    };

                    let pxar_writer = TokioWriter::new(writer);
                    create_archive(dir, pxar_writer, Flags::DEFAULT, |_| Ok(()), None, options)
                        .await
                }
                .await;
                if let Err(err) = result {
                    error!("pxar streaming task failed - {}", err);
                }
            });
        } else if format == "tar" {
            tokio::spawn(async move {
                let _inhibitor = _inhibitor;
                let _permit = _permit;
                if let Err(err) = tar_directory(&mut writer, &vm_path).await {
                    error!("file or dir streaming task failed - {}", err);
                }
            });
        } else {
            if format == "plain" && vm_path.is_dir() {
                bail!("cannot stream dir with format 'plain'");
            }
            tokio::spawn(async move {
                let _inhibitor = _inhibitor;
                let _permit = _permit;
                let result = async move {
                    if vm_path.is_dir() || format == "zip" {
                        zip_directory(&mut writer, &vm_path).await?;
                        Ok(())
                    } else if vm_path.is_file() {
                        let mut file = tokio::fs::OpenOptions::new()
                            .read(true)
                            .open(vm_path)
                            .await?;
                        tokio::io::copy(&mut file, &mut writer).await?;
                        Ok(())
                    } else {
                        bail!("invalid entry type for path: {:?}", vm_path);
                    }
                }
                .await;
                if let Err(err) = result {
                    error!("file or dir streaming task failed - {}", err);
                }
            });
        }

        let stream = tokio_util::io::ReaderStream::new(reader);

        let body = if param["zstd"].as_bool().unwrap_or(false) {
            let stream = ZstdEncoder::new(stream)?;
            Body::wrap_stream(stream)
        } else {
            Body::wrap_stream(stream)
        };
        Ok(Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/octet-stream")
            .body(body)
            .unwrap())
    }
    .boxed()
}
