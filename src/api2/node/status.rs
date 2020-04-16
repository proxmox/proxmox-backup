use failure::*;
use serde_json::{json, Value};

use proxmox::sys::linux::procfs;

use proxmox::api::{api, ApiMethod, Router, RpcEnvironment, SubdirMap, Permission};
use proxmox::list_subdirs_api_method;

use crate::api2::types::*;
use crate::config::acl::PRIV_SYS_AUDIT;

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
        permission: &Permission::Privilege(&[], PRIV_SYS_AUDIT, false),
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

pub const USAGE_ROUTER: Router = Router::new()
    .get(&API_METHOD_GET_USAGE);

pub const SUBDIRS: SubdirMap = &[
    ("usage", &USAGE_ROUTER),
];
pub const ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(SUBDIRS))
    .subdirs(SUBDIRS);
