use std::path::Path;
use std::process::Command;

use anyhow::{bail, format_err, Error};
use serde_json::Value;

use proxmox_sys::linux::procfs;

use proxmox_router::{ApiMethod, Permission, Router, RpcEnvironment};
use proxmox_schema::api;

use pbs_api_types::{NodePowerCommand, NODE_SCHEMA, PRIV_SYS_AUDIT, PRIV_SYS_POWER_MANAGEMENT};

use crate::api2::types::{
    NodeCpuInformation, NodeInformation, NodeMemoryCounters, NodeStatus, NodeSwapCounters,
};

impl std::convert::From<procfs::ProcFsCPUInfo> for NodeCpuInformation {
    fn from(info: procfs::ProcFsCPUInfo) -> Self {
        Self {
            model: info.model,
            sockets: info.sockets,
            cpus: info.cpus,
        }
    }
}

#[api(
    input: {
        properties: {
            node: {
                schema: NODE_SCHEMA,
            },
        },
    },
    returns: {
        type: NodeStatus,
    },
    access: {
        permission: &Permission::Privilege(&["system", "status"], PRIV_SYS_AUDIT, false),
    },
)]
/// Read node memory, CPU and (root) disk usage
fn get_status(
    _param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<NodeStatus, Error> {
    let meminfo: procfs::ProcFsMemInfo = procfs::read_meminfo()?;
    let memory = NodeMemoryCounters {
        total: meminfo.memtotal,
        used: meminfo.memused,
        free: meminfo.memfree,
    };

    let swap = NodeSwapCounters {
        total: meminfo.swaptotal,
        used: meminfo.swapused,
        free: meminfo.swapfree,
    };

    let kstat: procfs::ProcFsStat = procfs::read_proc_stat()?;
    let cpu = kstat.cpu;
    let wait = kstat.iowait_percent;

    let loadavg = procfs::Loadavg::read()?;
    let loadavg = [loadavg.one(), loadavg.five(), loadavg.fifteen()];

    let cpuinfo = procfs::read_cpuinfo()?;
    let cpuinfo = cpuinfo.into();

    let uname = nix::sys::utsname::uname();
    let kversion = format!(
        "{} {} {}",
        uname.sysname(),
        uname.release(),
        uname.version()
    );

    Ok(NodeStatus {
        memory,
        swap,
        root: crate::tools::disks::disk_usage(Path::new("/"))?,
        uptime: procfs::read_proc_uptime()?.0 as u64,
        loadavg,
        kversion,
        cpuinfo,
        cpu,
        wait,
        info: NodeInformation {
            fingerprint: crate::cert_info()?.fingerprint()?,
        },
    })
}

#[api(
    protected: true,
    input: {
        properties: {
            node: {
                schema: NODE_SCHEMA,
            },
            command: {
                type: NodePowerCommand,
            },
        }
    },
    access: {
        permission: &Permission::Privilege(&["system", "status"], PRIV_SYS_POWER_MANAGEMENT, false),
    },
)]
/// Reboot or shutdown the node.
fn reboot_or_shutdown(command: NodePowerCommand) -> Result<(), Error> {
    let systemctl_command = match command {
        NodePowerCommand::Reboot => "reboot",
        NodePowerCommand::Shutdown => "poweroff",
    };

    let output = Command::new("systemctl")
        .arg(systemctl_command)
        .output()
        .map_err(|err| format_err!("failed to execute systemctl - {}", err))?;

    if !output.status.success() {
        match output.status.code() {
            Some(code) => {
                let msg = String::from_utf8(output.stderr)
                    .map(|m| {
                        if m.is_empty() {
                            String::from("no error message")
                        } else {
                            m
                        }
                    })
                    .unwrap_or_else(|_| String::from("non utf8 error message (suppressed)"));
                bail!("diff failed with status code: {} - {}", code, msg);
            }
            None => bail!("systemctl terminated by signal"),
        }
    }
    Ok(())
}

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_GET_STATUS)
    .post(&API_METHOD_REBOOT_OR_SHUTDOWN);
