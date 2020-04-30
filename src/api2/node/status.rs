use std::process::Command;

use anyhow::{Error, format_err, bail};
use serde_json::{json, Value};

use proxmox::sys::linux::procfs;

use proxmox::api::{api, ApiMethod, Router, RpcEnvironment, Permission};

use crate::api2::types::*;
use crate::config::acl::{PRIV_SYS_AUDIT, PRIV_SYS_POWER_MANAGEMENT};

#[api(
    input: {
        properties: {
            node: {
                schema: NODE_SCHEMA,
            },
        },
    },
    returns: {
        type: Object,
        description: "Returns node memory, CPU and (root) disk usage",
        properties: {
            memory: {
                type: Object,
                description: "node memory usage counters",
                properties: {
                    total: {
                        description: "total memory",
                        type: Integer,
                    },
                    used: {
                        description: "total memory",
                        type: Integer,
                    },
                    free: {
                        description: "free memory",
                        type: Integer,
                    },
                },
            },
            cpu: {
                type: Number,
                description: "Total CPU usage since last query.",
                optional: true,
            },
        }
    },
    access: {
        permission: &Permission::Privilege(&["system", "status"], PRIV_SYS_AUDIT, false),
    },
)]
/// Read node memory, CPU and (root) disk usage
fn get_usage(
    _param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let meminfo: procfs::ProcFsMemInfo = procfs::read_meminfo()?;
    let kstat: procfs::ProcFsStat = procfs::read_proc_stat()?;

    Ok(json!({
        "memory": {
            "total": meminfo.memtotal,
            "used": meminfo.memused,
            "free": meminfo.memfree,
        },
        "cpu": kstat.cpu,
    }))
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

    let output = Command::new("/bin/systemctl")
        .arg(systemctl_command)
        .output()
        .map_err(|err| format_err!("failed to execute systemctl - {}", err))?;

    if !output.status.success() {
        match output.status.code() {
            Some(code) => {
                let msg = String::from_utf8(output.stderr)
                    .map(|m| if m.is_empty() { String::from("no error message") } else { m })
                    .unwrap_or_else(|_| String::from("non utf8 error message (suppressed)"));
                bail!("diff failed with status code: {} - {}", code, msg);
            }
            None => bail!("systemctl terminated by signal"),
        }
    }
    Ok(())
}

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_GET_USAGE)
    .post(&API_METHOD_REBOOT_OR_SHUTDOWN);
