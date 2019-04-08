use failure::*;
use lazy_static::lazy_static;
use regex::Regex;
use chrono::Local;

use std::sync::atomic::{AtomicUsize, Ordering, ATOMIC_USIZE_INIT};

use crate::tools;

/// Unique Process/Task Identifier
///
/// We use this to uniquely identify worker task. UPIDs have a short
/// string repesentaion, which gives additional information about the
/// type of the task. for example:
/// ```text
/// UPID:{node}:{pid}:{pstart}:{task_id}:{starttime}:{worker_type}:{worker_id}:{username}:
/// UPID:elsa:00004F37:0039E469:00000000:5CA78B83:garbage_collection::root@pam:
/// ```
/// Please note that we use tokio, so a single thread can run multiple
/// tasks.
#[derive(Debug, Clone)]
pub struct UPID {
    /// The Unix PID
    pub pid: libc::pid_t,
    /// The Unix process start time from `/proc/pid/stat`
    pub pstart: u64,
    /// The task start time (Epoch)
    pub starttime: i64,
    /// The task ID (inside the process/thread)
    pub task_id: usize,
    /// Worker type (arbitrary ASCII string)
    pub worker_type: String,
    /// Worker ID (arbitrary ASCII string)
    pub worker_id: Option<String>,
    /// The user who started the task
    pub username: String,
    /// The node name.
    pub node: String,
}

impl UPID {

    /// Create a new UPID
    pub fn new(worker_type: &str, worker_id: Option<String>, username: &str) -> Result<Self, Error> {

        let pid = unsafe { libc::getpid() };

        static WORKER_TASK_NEXT_ID: AtomicUsize = ATOMIC_USIZE_INIT;

        let task_id = WORKER_TASK_NEXT_ID.fetch_add(1, Ordering::SeqCst);

        Ok(UPID {
            pid,
            pstart: tools::procfs::read_proc_starttime(pid)?,
            starttime: Local::now().timestamp(),
            task_id,
            worker_type: worker_type.to_owned(),
            worker_id,
            username: username.to_owned(),
            node: tools::nodename().to_owned(),
        })
    }

    /// Returns the absolute path to the task log file
    pub fn log_path(&self) -> std::path::PathBuf {
        let mut path = std::path::PathBuf::from(super::PROXMOX_BACKUP_TASK_DIR);
        path.push(format!("{:02X}", self.pstart % 256));
        path.push(self.to_string());
        path
    }
}


impl std::str::FromStr for UPID {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {

        lazy_static! {
            static ref REGEX: Regex = Regex::new(concat!(
                r"^UPID:(?P<node>[a-zA-Z0-9]([a-zA-Z0-9\-]*[a-zA-Z0-9])?):(?P<pid>[0-9A-Fa-f]{8}):",
                r"(?P<pstart>[0-9A-Fa-f]{8,9}):(?P<task_id>[0-9A-Fa-f]{8,16}):(?P<starttime>[0-9A-Fa-f]{8}):",
                r"(?P<wtype>[^:\s]+):(?P<wid>[^:\s]*):(?P<username>[^:\s]+):$"
            )).unwrap();
        }

        if let Some(cap) = REGEX.captures(s) {

            return Ok(UPID {
                pid: i32::from_str_radix(&cap["pid"], 16).unwrap(),
                pstart: u64::from_str_radix(&cap["pstart"], 16).unwrap(),
                starttime: i64::from_str_radix(&cap["starttime"], 16).unwrap(),
                task_id: usize::from_str_radix(&cap["task_id"], 16).unwrap(),
                worker_type: cap["wtype"].to_string(),
                worker_id: if cap["wid"].is_empty() { None } else { Some(cap["wid"].to_string()) },
                username: cap["username"].to_string(),
                node: cap["node"].to_string(),
            });
        } else {
            bail!("unable to parse UPID '{}'", s);
        }

    }
}

impl std::fmt::Display for UPID {

    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {

        let wid = if let Some(ref id) = self.worker_id { id } else { "" };

        // Note: pstart can be > 32bit if uptime > 497 days, so this can result in
        // more that 8 characters for pstart

        write!(f, "UPID:{}:{:08X}:{:08X}:{:08X}:{:08X}:{}:{}:{}:",
               self.node, self.pid, self.pstart, self.task_id, self.starttime, self.worker_type, wid, self.username)
    }
}
