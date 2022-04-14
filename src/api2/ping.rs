//! Cheap check if the API daemon is online.

use anyhow::Error;
use serde_json::{json, Value};

use proxmox_router::{Permission, Router};
use proxmox_schema::api;

#[api(
    returns: {
        description: "Dummy ping",
        type: Object,
        properties: {
            pong: {
                description: "Always true",
                type: bool,
            }
        }
    },
    access: {
        description: "Anyone can access this, because it's used for a cheap check if the API daemon is online.",
        permission: &Permission::World,
    }
)]
/// Dummy method which replies with `{ "pong": True }`
pub fn ping() -> Result<Value, Error> {
    Ok(json!({
        "pong": true,
    }))
}
pub const ROUTER: Router = Router::new().get(&API_METHOD_PING);
