//! Proxmox Server/Service framework
//!
//! This code provides basic primitives to build our REST API
//! services. We want async IO, so this is built on top of
//! tokio/hyper.

use lazy_static::lazy_static;
use nix::unistd::Pid;

use proxmox::sys::linux::procfs::PidStat;

use crate::buildcfg;

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

pub fn ctrl_sock_from_pid(pid: i32) -> String {
    format!("\0{}/control-{}.sock", buildcfg::PROXMOX_BACKUP_RUN_DIR, pid)
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
