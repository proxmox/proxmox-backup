use anyhow::Error;
use libc::pid_t;
use nix::unistd::Pid;
use std::iter::Sum;
use std::path::PathBuf;

use pbs_api_types::Operation;
use proxmox_sys::fs::{file_read_optional_string, open_file_locked, replace_file, CreateOptions};
use proxmox_sys::linux::procfs;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Clone, Copy, Default)]
pub struct ActiveOperationStats {
    pub read: i64,
    pub write: i64,
}

impl Sum<Self> for ActiveOperationStats {
    fn sum<I>(iter: I) -> Self
    where
        I: Iterator<Item = Self>,
    {
        iter.fold(Self::default(), |a, b| Self {
            read: a.read + b.read,
            write: a.write + b.write,
        })
    }
}

#[derive(Deserialize, Serialize, Clone)]
struct TaskOperations {
    pid: u32,
    starttime: u64,
    active_operations: ActiveOperationStats,
}

pub fn get_active_operations(name: &str) -> Result<ActiveOperationStats, Error> {
    let path = PathBuf::from(format!("{}/{}", crate::ACTIVE_OPERATIONS_DIR, name));

    Ok(match file_read_optional_string(&path)? {
        Some(data) => serde_json::from_str::<Vec<TaskOperations>>(&data)?
            .iter()
            .filter_map(
                |task| match procfs::check_process_running(task.pid as pid_t) {
                    Some(stat) if task.starttime == stat.starttime => Some(task.active_operations),
                    _ => None,
                },
            )
            .sum(),
        None => ActiveOperationStats::default(),
    })
}

pub fn update_active_operations(name: &str, operation: Operation, count: i64) -> Result<(), Error> {
    let path = PathBuf::from(format!("{}/{}", crate::ACTIVE_OPERATIONS_DIR, name));
    let lock_path = PathBuf::from(format!("{}/{}.lock", crate::ACTIVE_OPERATIONS_DIR, name));

    let user = pbs_config::backup_user()?;
    let options = CreateOptions::new()
        .group(user.gid)
        .owner(user.uid)
        .perm(nix::sys::stat::Mode::from_bits_truncate(0o660));

    let timeout = std::time::Duration::new(10, 0);
    open_file_locked(&lock_path, timeout, true, options.clone())?;

    let pid = std::process::id();
    let starttime = procfs::PidStat::read_from_pid(Pid::from_raw(pid as pid_t))?.starttime;
    let mut updated = false;

    let mut updated_tasks: Vec<TaskOperations> = match file_read_optional_string(&path)? {
        Some(data) => serde_json::from_str::<Vec<TaskOperations>>(&data)?
            .iter_mut()
            .filter_map(
                |task| match procfs::check_process_running(task.pid as pid_t) {
                    Some(stat) if pid == task.pid && stat.starttime != task.starttime => None,
                    Some(_) => {
                        if pid == task.pid {
                            updated = true;
                            match operation {
                                Operation::Read => task.active_operations.read += count,
                                Operation::Write => task.active_operations.write += count,
                            };
                        }
                        Some(task.clone())
                    }
                    _ => None,
                },
            )
            .collect(),
        None => Vec::new(),
    };

    if !updated {
        updated_tasks.push(TaskOperations {
            pid,
            starttime,
            active_operations: match operation {
                Operation::Read => ActiveOperationStats { read: 1, write: 0 },
                Operation::Write => ActiveOperationStats { read: 0, write: 1 },
            },
        })
    }
    replace_file(
        &path,
        serde_json::to_string(&updated_tasks)?.as_bytes(),
        options,
        false,
    )
}
