//! Proxmox Server/Service framework
//!
//! This code provides basic primitives to build our REST API
//! services. We want async IO, so this is built on top of
//! tokio/hyper.

use anyhow::{format_err, Error};
use lazy_static::lazy_static;
use nix::unistd::Pid;
use serde_json::Value;

use proxmox::sys::linux::procfs::PidStat;
use proxmox::tools::fs::{create_path, CreateOptions};

use pbs_buildcfg;

lazy_static! {
    static ref PID: i32 = unsafe { libc::getpid() };
    static ref PSTART: u64 = PidStat::read_from_pid(Pid::from_raw(*PID)).unwrap().starttime;
}

pub fn pid() -> i32 {
    *PID
}

pub fn pstart() -> u64 {
    *PSTART
}

pub fn write_pid(pid_fn: &str) -> Result<(), Error> {
    let pid_str = format!("{}\n", *PID);
    proxmox::tools::fs::replace_file(pid_fn, pid_str.as_bytes(), CreateOptions::new())
}

pub fn read_pid(pid_fn: &str) -> Result<i32, Error> {
    let pid = proxmox::tools::fs::file_get_contents(pid_fn)?;
    let pid = std::str::from_utf8(&pid)?.trim();
    pid.parse().map_err(|err| format_err!("could not parse pid - {}", err))
}

pub fn ctrl_sock_from_pid(pid: i32) -> String {
    format!("\0{}/control-{}.sock", pbs_buildcfg::PROXMOX_BACKUP_RUN_DIR, pid)
}

pub fn our_ctrl_sock() -> String {
    ctrl_sock_from_pid(*PID)
}

mod environment;
pub use environment::*;

mod upid;
pub use upid::*;

mod state;
pub use state::*;

mod command_socket;
pub use command_socket::*;

mod worker_task;
pub use worker_task::*;

mod h2service;
pub use h2service::*;

pub mod config;
pub use config::*;

pub mod formatter;

#[macro_use]
pub mod rest;

pub mod jobstate;

mod verify_job;
pub use verify_job::*;

mod prune_job;
pub use prune_job::*;

mod gc_job;
pub use gc_job::*;

mod email_notifications;
pub use email_notifications::*;

mod report;
pub use report::*;

pub mod ticket;

pub mod auth;

pub mod pull;

pub(crate) async fn reload_proxy_certificate() -> Result<(), Error> {
    let proxy_pid = crate::server::read_pid(pbs_buildcfg::PROXMOX_BACKUP_PROXY_PID_FN)?;
    let sock = crate::server::ctrl_sock_from_pid(proxy_pid);
    let _: Value = crate::server::send_raw_command(sock, "{\"command\":\"reload-certificate\"}\n")
        .await?;
    Ok(())
}

pub(crate) async fn notify_datastore_removed() -> Result<(), Error> {
    let proxy_pid = crate::server::read_pid(pbs_buildcfg::PROXMOX_BACKUP_PROXY_PID_FN)?;
    let sock = crate::server::ctrl_sock_from_pid(proxy_pid);
    let _: Value = crate::server::send_raw_command(sock, "{\"command\":\"datastore-removed\"}\n")
        .await?;
    Ok(())
}

/// Create the base run-directory.
///
/// This exists to fixate the permissions for the run *base* directory while allowing intermediate
/// directories after it to have different permissions.
pub fn create_run_dir() -> Result<(), Error> {
    let backup_user = crate::backup::backup_user()?;
    let opts = CreateOptions::new()
        .owner(backup_user.uid)
        .group(backup_user.gid);
    let _: bool = create_path(pbs_buildcfg::PROXMOX_BACKUP_RUN_DIR_M!(), None, Some(opts))?;
    Ok(())
}
