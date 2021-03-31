///! File-restore API running inside the restore VM
use anyhow::{bail, Error};
use std::ffi::OsStr;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use proxmox::api::{api, ApiMethod, Permission, Router, RpcEnvironment, SubdirMap};
use proxmox::list_subdirs_api_method;

use proxmox_backup::api2::types::*;
use proxmox_backup::backup::DirEntryAttribute;
use proxmox_backup::tools::fs::read_subdir;

use super::{disk::ResolveResult, watchdog_remaining, watchdog_ping};

// NOTE: All API endpoints must have Permission::Superuser, as the configs for authentication do
// not exist within the restore VM. Safety is guaranteed by checking a ticket via a custom ApiAuth.

const SUBDIRS: SubdirMap = &[
    ("list", &Router::new().get(&API_METHOD_LIST)),
    ("status", &Router::new().get(&API_METHOD_STATUS)),
    ("stop", &Router::new().get(&API_METHOD_STOP)),
];

pub const ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(SUBDIRS))
    .subdirs(SUBDIRS);

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
fn status(rpcenv: &mut dyn RpcEnvironment, keep_timeout: bool) -> Result<RestoreDaemonStatus, Error> {
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

    let mut disk_state = crate::DISK_STATE.lock().unwrap();
    let query_result = disk_state.resolve(&param_path_buf)?;

    match query_result {
        ResolveResult::Path(vm_path) => {
            let root_entry = get_dir_entry(&vm_path)?;
            match root_entry {
                DirEntryAttribute::File { .. } => {
                    // list on file, return details
                    res.push(ArchiveEntry::new(&param_path, &root_entry));
                }
                DirEntryAttribute::Directory { .. } => {
                    // list on directory, return all contained files/dirs
                    for f in read_subdir(libc::AT_FDCWD, &vm_path)? {
                        if let Ok(f) = f {
                            let name = f.file_name().to_bytes();
                            let path = &Path::new(OsStr::from_bytes(name));
                            if path.components().count() == 1 {
                                // ignore '.' and '..'
                                match path.components().next().unwrap() {
                                    std::path::Component::CurDir
                                    | std::path::Component::ParentDir => continue,
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
                                    &entry,
                                ));
                            }
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
                res.push(ArchiveEntry::new(
                    &t_path[..],
                    &DirEntryAttribute::Directory { start: 0 },
                ));
            }
        }
        ResolveResult::BucketComponents(comps) => {
            for c in comps {
                let mut c_path = path.clone();
                c_path.push(b'/');
                c_path.extend(c.as_bytes());
                res.push(ArchiveEntry::new(
                    &c_path[..],
                    &DirEntryAttribute::Directory { start: 0 },
                ));
            }
        }
    }

    Ok(res)
}
