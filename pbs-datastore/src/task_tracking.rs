use anyhow::Error;
use libc::pid_t;
use nix::unistd::Pid;
use std::path::PathBuf;

use pbs_api_types::Operation;
use proxmox_sys::fs::{file_read_optional_string, open_file_locked, replace_file, CreateOptions};
use proxmox_sys::linux::procfs;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Clone)]
struct TaskOperations {
    pid: u32,
    starttime: u64,
    reading_operations: i64,
    writing_operations: i64,
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
                                Operation::Read => task.reading_operations += count,
                                Operation::Write => task.writing_operations += count,
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
        updated_tasks.push(match operation {
            Operation::Read => TaskOperations {
                pid,
                starttime,
                reading_operations: 1,
                writing_operations: 0,
            },
            Operation::Write => TaskOperations {
                pid,
                starttime,
                reading_operations: 0,
                writing_operations: 1,
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
