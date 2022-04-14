//! Version information

use anyhow::Error;
use serde_json::{json, Value};

use proxmox_router::{ApiHandler, ApiMethod, Permission, Router, RpcEnvironment};
use proxmox_schema::ObjectSchema;

fn get_version(
    _param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {
    Ok(json!({
        "version": pbs_buildcfg::PROXMOX_PKG_VERSION,
        "release": pbs_buildcfg::PROXMOX_PKG_RELEASE,
        "repoid": pbs_buildcfg::PROXMOX_PKG_REPOID
    }))
}

pub const ROUTER: Router = Router::new().get(
    &ApiMethod::new(
        &ApiHandler::Sync(&get_version),
        &ObjectSchema::new("Proxmox Backup Server API version.", &[]),
    )
    .access(None, &Permission::Anybody),
);
