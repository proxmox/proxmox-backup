use failure::*;
use serde_json::{json, Value};

use proxmox::api::{api, Router, Permission};

use crate::tools;
use crate::config::acl::PRIV_SYS_AUDIT;

#[api(
    returns: {
        description: "Subscription status.",
        properties: {
            status: {
                type: String,
                description: "'NotFound', 'active' or 'inactive'."
            },
            message: {
                type: String,
                description: "Human readable problem description.",
            },
            serverid: {
                type: String,
                description: "The unique server ID.",
            },
            url: {
                type: String,
                description: "URL to Web Shop.",
            },
        },
    },
    access: {
        permission: &Permission::Privilege(&[], PRIV_SYS_AUDIT, false),
    },
)]
/// Read subscription info.
fn get_subscription(_param: Value) -> Result<Value, Error> {

    let url = "https://www.proxmox.com/en/proxmox-backup-server/pricing";
    Ok(json!({
        "status": "NotFound",
	"message": "There is no subscription key",
	"serverid": tools::get_hardware_address()?,
	"url":  url,
     }))
}

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_GET_SUBSCRIPTION);
