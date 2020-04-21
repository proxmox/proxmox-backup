use anyhow::{Error};
use serde_json::{Value, to_value};

use proxmox::api::{api, ApiMethod, Router, RpcEnvironment, Permission};

//use crate::api2::types::*;
use crate::config::network;
use crate::config::acl::{PRIV_SYS_AUDIT};

#[api(
    input: {
        properties: {},
    },
    returns: {
        description: "List network devices (with config digest).",
        type: Array,
        items: {
            type: network::Interface,
        },
    },
    access: {
        permission: &Permission::Privilege(&[], PRIV_SYS_AUDIT, false),
    },
)]
/// List all datastores
pub fn list_network_devices(
    _param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let (config, digest) = network::config()?;
    let digest = proxmox::tools::digest_to_hex(&digest);

    let mut list = Vec::new();

    for interface in config.interfaces.values() {
        let mut item: Value = to_value(interface)?;
        item["digest"] = digest.clone().into();
        list.push(item);
    }

    Ok(list.into())
}

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_LIST_NETWORK_DEVICES);
