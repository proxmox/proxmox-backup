use failure::*;
use serde_json::{json, Value};

use proxmox::api::{api, Router, Permission};

use crate::api2::types::*;
use crate::config::acl::{PRIV_SYS_AUDIT};

#[api(
    input: {
        properties: {
            node: {
                schema: NODE_SCHEMA,
            },
        },
    },
    returns: {
        description: "The network configuration from /etc/network/interfaces.",
        properties: {
            // fixme
        },
    },
    access: {
        permission: &Permission::Privilege(&[], PRIV_SYS_AUDIT, false),
    },
)]
/// Read network configuration.
fn get_network_config(
    _param: Value,
) -> Result<Value, Error> {

    Ok(json!({}))
}

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_GET_NETWORK_CONFIG);
  
