//! Block file access via a small QEMU restore VM using the PBS block driver in QEMU
use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs::{File, OpenOptions};
use std::io::{prelude::*, BufReader, BufWriter, SeekFrom};
use std::os::unix::prelude::OsStrExt;
use std::path::Path;

use anyhow::{bail, Error};
use futures::FutureExt;
use serde::{Deserialize, Serialize};
use serde_json::json;

use proxmox_sys::fs::lock_file;

use pbs_api_types::{file_restore::FileRestoreFormat, BackupDir, BackupNamespace};
use pbs_client::{BackupRepository, VsockClient, DEFAULT_VSOCK_PORT};
use pbs_datastore::catalog::ArchiveEntry;

use super::block_driver::*;
use crate::get_user_run_dir;
use crate::qemu_helper::{self, MAX_MEMORY_DIMM_SIZE};

const RESTORE_VM_MAP: &str = "restore-vm-map.json";

pub struct QemuBlockDriver {}

#[derive(Clone, Hash, Serialize, Deserialize)]
struct VMState {
    pid: i32,
    cid: i32,
    ticket: String,
}

struct VMStateMap {
    map: HashMap<String, VMState>,
    file: File,
}

impl VMStateMap {
    fn open_file_raw(write: bool) -> Result<File, Error> {
        use std::os::unix::fs::OpenOptionsExt;
        let mut path = get_user_run_dir()?;
        path.push(RESTORE_VM_MAP);
        OpenOptions::new()
            .read(true)
            .write(write)
            .create(write)
            .mode(0o600)
            .open(path)
            .map_err(Error::from)
    }

    /// Acquire a lock on the state map and retrieve a deserialized version
    fn load() -> Result<Self, Error> {
        let mut file = Self::open_file_raw(true)?;
        lock_file(&mut file, true, Some(std::time::Duration::from_secs(120)))?;
        let map = serde_json::from_reader(BufReader::new(&mut file)).unwrap_or_default();
        Ok(Self { map, file })
    }

    /// Load a read-only copy of the current VM map. Only use for informational purposes, like
    /// shell auto-completion, for anything requiring consistency use load() !
    fn load_read_only() -> Result<HashMap<String, VMState>, Error> {
        let file = Self::open_file_raw(false)?;
        Ok(serde_json::from_reader(BufReader::new(file)).unwrap_or_default())
    }

    /// Write back a potentially modified state map, consuming the held lock
    fn write(mut self) -> Result<(), Error> {
        self.file.seek(SeekFrom::Start(0))?;
        self.file.set_len(0)?;
        serde_json::to_writer(BufWriter::new(&mut self.file), &self.map)?;

        // drop ourselves including file lock
        Ok(())
    }

    /// Return the map, but drop the lock immediately
    fn read_only(self) -> HashMap<String, VMState> {
        self.map
    }
}

fn make_name(repo: &BackupRepository, ns: &BackupNamespace, snap: &BackupDir) -> String {
    let full = if ns.is_root() {
        format!("qemu_{repo}/{snap}")
    } else {
        format!("qemu_{repo}:{ns}/{snap}")
    };
    proxmox_sys::systemd::escape_unit(full, false)
}

/// remove non-responsive VMs from given map, returns 'true' if map was modified
async fn cleanup_map(map: &mut HashMap<String, VMState>) -> bool {
    let mut to_remove = Vec::new();
    for (name, state) in map.iter() {
        let client = VsockClient::new(state.cid, DEFAULT_VSOCK_PORT, Some(state.ticket.clone()));
        let res = client
            .get("api2/json/status", Some(json!({"keep-timeout": true})))
            .await;
        if res.is_err() {
            // VM is not reachable, remove from map and inform user
            to_remove.push(name.clone());
            log::warn!(
                "VM '{}' (pid: {}, cid: {}) was not reachable, removing from map",
                name,
                state.pid,
                state.cid
            );
            let _ = super::qemu_helper::try_kill_vm(state.pid);
        }
    }

    for tr in &to_remove {
        map.remove(tr);
    }

    !to_remove.is_empty()
}

fn new_ticket() -> String {
    proxmox_uuid::Uuid::generate().to_string()
}

async fn ensure_running(details: &SnapRestoreDetails) -> Result<(i32, VsockClient), Error> {
    let name = make_name(&details.repo, &details.namespace, &details.snapshot);
    let mut state = VMStateMap::load()?;

    cleanup_map(&mut state.map).await;

    let new_cid;
    let vms = match state.map.get(&name) {
        Some(vm) => {
            let client = VsockClient::new(vm.cid, DEFAULT_VSOCK_PORT, Some(vm.ticket.clone()));
            let res = client.get("api2/json/status", None).await;
            match res {
                Ok(_) => {
                    // VM is running and we just reset its timeout, nothing to do
                    return Ok((vm.cid, client));
                }
                Err(err) => {
                    log::warn!("stale VM detected, restarting ({})", err);
                    // VM is dead, restart
                    let _ = super::qemu_helper::try_kill_vm(vm.pid);
                    let vms = start_vm(vm.cid, details).await?;
                    new_cid = vms.cid;
                    state.map.insert(name, vms.clone());
                    vms
                }
            }
        }
        None => {
            let mut cid = state
                .map
                .iter()
                .map(|v| v.1.cid)
                .max()
                .unwrap_or(0)
                .wrapping_add(1);

            // offset cid by user id, to avoid unnecessary retries
            let running_uid = nix::unistd::Uid::current();
            cid = cid.wrapping_add(running_uid.as_raw() as i32);

            // some low CIDs have special meaning, start at 10 to avoid them
            cid = cid.max(10);

            let vms = start_vm(cid, details).await?;
            new_cid = vms.cid;
            state.map.insert(name, vms.clone());
            vms
        }
    };

    state.write()?;
    Ok((
        new_cid,
        VsockClient::new(new_cid, DEFAULT_VSOCK_PORT, Some(vms.ticket)),
    ))
}

fn path_is_zfs(path: &[u8]) -> bool {
    if path.is_empty() {
        return false;
    }
    let path = Path::new(OsStr::from_bytes(path));
    let mut components = path.components();
    let part = match components.next() {
        Some(std::path::Component::RootDir) => match components.next() {
            Some(std::path::Component::Normal(comp)) => comp,
            _ => return false,
        },
        Some(std::path::Component::Normal(comp)) => comp,
        _ => return false,
    };

    part == OsStr::new("zpool") && components.next().is_some()
}

async fn handle_extra_guest_memory_needs(cid: i32, path: &[u8]) {
    use std::env::var;
    match var("PBS_FILE_RESTORE_MEM_HOTPLUG_ALLOW").ok().as_deref() {
        Some("true") => (),
        _ => return, // this is opt-in
    }
    let size = match var("PBS_FILE_RESTORE_MEM_HOTPLUG_SIZE_MB").map(|v| v.parse::<usize>()) {
        Ok(Ok(size)) if size > MAX_MEMORY_DIMM_SIZE => {
            log::warn!("limit memory request of {size} to {MAX_MEMORY_DIMM_SIZE}");
            MAX_MEMORY_DIMM_SIZE
        }
        Ok(Ok(size)) => size,
        _ => 256, // in practice this means a total of ~ 512 MB depending on disk count
    };

    if path_is_zfs(path) {
        if let Err(err) = qemu_helper::hotplug_memory(cid, size).await {
            log::error!("could not increase memory: {err}");
        }
    }
}

async fn start_vm(cid_request: i32, details: &SnapRestoreDetails) -> Result<VMState, Error> {
    let ticket = new_ticket();
    let files = details
        .manifest
        .files()
        .iter()
        .map(|file| file.filename.clone())
        .filter(|name| name.ends_with(".img.fidx"));
    let (pid, cid) =
        super::qemu_helper::start_vm((cid_request.abs() & 0xFFFF) as u16, details, files, &ticket)
            .await?;
    Ok(VMState { pid, cid, ticket })
}

impl BlockRestoreDriver for QemuBlockDriver {
    fn data_list(
        &self,
        details: SnapRestoreDetails,
        img_file: String,
        mut path: Vec<u8>,
    ) -> Async<Result<Vec<ArchiveEntry>, Error>> {
        async move {
            let (cid, client) = ensure_running(&details).await?;
            if !path.is_empty() && path[0] != b'/' {
                path.insert(0, b'/');
            }
            handle_extra_guest_memory_needs(cid, &path).await;
            let path = base64::encode(img_file.bytes().chain(path).collect::<Vec<u8>>());
            let mut result = client
                .get("api2/json/list", Some(json!({ "path": path })))
                .await?;
            serde_json::from_value(result["data"].take()).map_err(|err| err.into())
        }
        .boxed()
    }

    fn data_extract(
        &self,
        details: SnapRestoreDetails,
        img_file: String,
        mut path: Vec<u8>,
        format: Option<FileRestoreFormat>,
        zstd: bool,
    ) -> Async<Result<Box<dyn tokio::io::AsyncRead + Unpin + Send>, Error>> {
        async move {
            let (cid, client) = ensure_running(&details).await?;
            if !path.is_empty() && path[0] != b'/' {
                path.insert(0, b'/');
            }
            handle_extra_guest_memory_needs(cid, &path).await;
            let path = base64::encode(img_file.bytes().chain(path).collect::<Vec<u8>>());
            let (mut tx, rx) = tokio::io::duplex(1024 * 4096);
            let mut data = json!({ "path": path, "zstd": zstd });
            if let Some(format) = format {
                data["format"] = serde_json::to_value(format)?;
            }
            tokio::spawn(async move {
                if let Err(err) = client
                    .download("api2/json/extract", Some(data), &mut tx)
                    .await
                {
                    log::error!("reading file extraction stream failed - {}", err);
                    std::process::exit(1);
                }
            });

            Ok(Box::new(rx) as Box<dyn tokio::io::AsyncRead + Unpin + Send>)
        }
        .boxed()
    }

    fn status(&self) -> Async<Result<Vec<DriverStatus>, Error>> {
        async move {
            let mut state_map = VMStateMap::load()?;
            let modified = cleanup_map(&mut state_map.map).await;
            let map = if modified {
                let m = state_map.map.clone();
                state_map.write()?;
                m
            } else {
                state_map.read_only()
            };
            let mut result = Vec::new();

            for (n, s) in map.iter() {
                let client = VsockClient::new(s.cid, DEFAULT_VSOCK_PORT, Some(s.ticket.clone()));
                let resp = client
                    .get("api2/json/status", Some(json!({"keep-timeout": true})))
                    .await;
                let name = proxmox_sys::systemd::unescape_unit(n)
                    .unwrap_or_else(|_| "<invalid name>".to_owned());
                let mut extra = json!({"pid": s.pid, "cid": s.cid});

                match resp {
                    Ok(status) => match status["data"].as_object() {
                        Some(map) => {
                            for (k, v) in map.iter() {
                                extra[k] = v.clone();
                            }
                        }
                        None => {
                            let err = format!("invalid JSON received from /status call: {status}");
                            extra["error"] = json!(err);
                        }
                    },
                    Err(err) => {
                        let err = format!("error during /status API call: {err}");
                        extra["error"] = json!(err);
                    }
                }

                result.push(DriverStatus {
                    id: name,
                    data: extra,
                });
            }

            Ok(result)
        }
        .boxed()
    }

    fn stop(&self, id: String) -> Async<Result<(), Error>> {
        async move {
            let name = proxmox_sys::systemd::escape_unit(&id, false);
            let mut map = VMStateMap::load()?;
            let map_mod = cleanup_map(&mut map.map).await;
            match map.map.get(&name) {
                Some(state) => {
                    let client =
                        VsockClient::new(state.cid, DEFAULT_VSOCK_PORT, Some(state.ticket.clone()));
                    // ignore errors, this either fails because:
                    // * the VM is unreachable/dead, in which case we don't want it in the map
                    // * the call was successful and the connection reset when the VM stopped
                    let _ = client.get("api2/json/stop", None).await;
                    map.map.remove(&name);
                    map.write()?;
                }
                None => {
                    if map_mod {
                        map.write()?;
                    }
                    bail!("VM with name '{name}' not found");
                }
            }
            Ok(())
        }
        .boxed()
    }

    fn list(&self) -> Vec<String> {
        match VMStateMap::load_read_only() {
            Ok(state) => state
                .iter()
                .filter_map(|(name, _)| proxmox_sys::systemd::unescape_unit(name).ok())
                .collect(),
            Err(_) => Vec::new(),
        }
    }
}
