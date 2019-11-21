use failure::*;

use crate::api_schema::*;
use crate::api_schema::router::*;
use serde_json::{json, Value};

pub const PROXMOX_PKG_VERSION: &str =
    concat!(
        env!("CARGO_PKG_VERSION_MAJOR"),
        ".",
        env!("CARGO_PKG_VERSION_MINOR"),
    );
pub const PROXMOX_PKG_RELEASE: &str = env!("CARGO_PKG_VERSION_PATCH");
pub const PROXMOX_PKG_REPOID: &str = env!("CARGO_PKG_REPOSITORY");

fn get_version(
    _param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    Ok(json!({
        "version": PROXMOX_PKG_VERSION,
        "release": PROXMOX_PKG_RELEASE,
        "repoid": PROXMOX_PKG_REPOID
    }))
}

pub const ROUTER: Router = Router::new()
    .get(
        &ApiMethod::new(
            &ApiHandler::Sync(&get_version),
            &ObjectSchema::new("Proxmox Backup Server API version.", &[])
        )
    );

