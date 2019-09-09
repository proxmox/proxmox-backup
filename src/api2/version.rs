use failure::*;

use crate::api_schema::*;
use crate::api_schema::router::*;
use serde_json::{json, Value};

pub const PROXMOX_PKG_VERSION: &'static str =
    concat!(
        env!("CARGO_PKG_VERSION_MAJOR"),
        ".",
        env!("CARGO_PKG_VERSION_MINOR"),
    );
pub const PROXMOX_PKG_RELEASE: &'static str = env!("CARGO_PKG_VERSION_PATCH");
pub const PROXMOX_PKG_REPOID: &'static str = env!("CARGO_PKG_REPOSITORY");

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

pub fn router() -> Router {

    let route = Router::new()
        .get(ApiMethod::new(
            get_version,
            ObjectSchema::new("Proxmox Backup Server API version.")));

    route
}
