///! File-restore API running inside the restore VM
use anyhow::Error;
use serde_json::Value;
use std::fs;

use proxmox::api::{api, ApiMethod, Permission, Router, RpcEnvironment, SubdirMap};
use proxmox::list_subdirs_api_method;

use proxmox_backup::api2::types::*;

use super::{watchdog_remaining, watchdog_ping};

// NOTE: All API endpoints must have Permission::Superuser, as the configs for authentication do
// not exist within the restore VM. Safety is guaranteed by checking a ticket via a custom ApiAuth.

const SUBDIRS: SubdirMap = &[
    ("status", &Router::new().get(&API_METHOD_STATUS)),
    ("stop", &Router::new().get(&API_METHOD_STOP)),
];

pub const ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(SUBDIRS))
    .subdirs(SUBDIRS);

fn read_uptime() -> Result<f32, Error> {
    let uptime = fs::read_to_string("/proc/uptime")?;
    // unwrap the Option, if /proc/uptime is empty we have bigger problems
    Ok(uptime.split_ascii_whitespace().next().unwrap().parse()?)
}

#[api(
    input: {
        properties: {
            "keep-timeout": {
                type: bool,
                description: "If true, do not reset the watchdog timer on this API call.",
                default: false,
                optional: true,
            },
        },
    },
    access: {
        description: "Permissions are handled outside restore VM. This call can be made without a ticket, but keep-timeout is always assumed 'true' then.",
        permission: &Permission::World,
    },
    returns: {
        type: RestoreDaemonStatus,
    }
)]
/// General status information
fn status(rpcenv: &mut dyn RpcEnvironment, keep_timeout: bool) -> Result<RestoreDaemonStatus, Error> {
    if !keep_timeout && rpcenv.get_auth_id().is_some() {
        watchdog_ping();
    }
    Ok(RestoreDaemonStatus {
        uptime: read_uptime()? as i64,
        timeout: watchdog_remaining(),
    })
}

#[api(
    access: {
        description: "Permissions are handled outside restore VM.",
        permission: &Permission::Superuser,
    },
)]
/// Stop the restore VM immediately, this will never return if successful
fn stop() {
    use nix::sys::reboot;
    println!("/stop called, shutting down");
    let err = reboot::reboot(reboot::RebootMode::RB_POWER_OFF).unwrap_err();
    println!("'reboot' syscall failed: {}", err);
    std::process::exit(1);
}
