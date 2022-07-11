use std::ffi::CStr;
use std::path::PathBuf;

use anyhow::{format_err, Error};
use tokio::task::spawn_blocking;

/// `proxmox_sys::fs::fs_into` wrapped in a `spawn_blocking` call.
pub async fn fs_info(path: PathBuf) -> Result<proxmox_sys::fs::FileSystemInformation, Error> {
    Ok(spawn_blocking(move || proxmox_sys::fs::fs_info(&path))
        .await
        .map_err(|err| format_err!("error waiting for fs_info call: {err}"))??)
}

/// `proxmox_sys::fs::fs_into` wrapped in a `spawn_blocking` call.
///
/// We cannot use `&'static CStr` in the above as we get from `proxmox_lang::c_str!` because
/// `NixPath` is only implemented directly on `CStr`, not on `&CStr`.
pub async fn fs_info_static(
    path: &'static CStr,
) -> Result<proxmox_sys::fs::FileSystemInformation, Error> {
    Ok(spawn_blocking(move || proxmox_sys::fs::fs_info(path))
        .await
        .map_err(|err| format_err!("error waiting for fs_info call: {err}"))??)
}
