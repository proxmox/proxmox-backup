use anyhow::Error;
use serde_json::{json, Value};

use proxmox_router::{Router, Permission};
use proxmox_schema::api;

use pbs_api_types::{
    TRAFFIC_CONTROL_ID_SCHEMA, PRIV_SYS_AUDIT,
};

use crate::TRAFFIC_CONTROL_CACHE;

#[api(
    input: {
        properties: {},
    },
    returns: {
        description: "Show current traffic control rates.",
        type: Array,
        items: {
            description: "Current rates per rule.",
            properties: {
                name: {
                    schema: TRAFFIC_CONTROL_ID_SCHEMA,
                },
                "rate-in": {
                    description: "Current ingress rate in bytes/second",
                    type: u64,
                },
                "rate-out": {
                    description: "Current egress rate in bytes/second",
                    type: u64,
                },
            },
        },
    },
    access: {
        permission: &Permission::Privilege(&[], PRIV_SYS_AUDIT, false),
    },
)]
/// Show current traffic for all traffic control rules.
pub fn show_current_traffic() -> Result<Value, Error> {

    let mut list = Vec::new();

    let cache = TRAFFIC_CONTROL_CACHE.lock().unwrap();
    for (rule, stat) in cache.current_rate_map().iter() {
        list.push(json!({
            "name": rule,
            "rate-in": stat.rate_in,
            "rate-out": stat.rate_out,
        }));
    }

    Ok(list.into())
}

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_SHOW_CURRENT_TRAFFIC);
