use anyhow::Error;
use serde_json::{json, Value};

use proxmox_router::{ApiMethod, Permission, Router, RpcEnvironment};
use proxmox_schema::api;

use pbs_api_types::{NODE_SCHEMA, PRIV_SYS_AUDIT};

use crate::server::generate_report;

#[api(
    input: {
        properties: {
            node: {
                schema: NODE_SCHEMA,
            },
        },
    },
    returns: {
        type: String,
        description: "Returns report of the node"
    },
    access: {
        permission: &Permission::Privilege(&["system", "status"], PRIV_SYS_AUDIT, false),
    },
)]
/// Generate a report
fn get_report(
    _param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {
    Ok(json!(generate_report()))
}

pub const ROUTER: Router = Router::new().get(&API_METHOD_GET_REPORT);
