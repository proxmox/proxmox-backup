use anyhow::{Error};
use serde_json::{json, Value};

use proxmox::api::{api, Router, RpcEnvironment, Permission};

use crate::tools;
use crate::config::acl::PRIV_SYS_AUDIT;
use crate::config::cached_user_info::CachedUserInfo;
use crate::api2::types::{NODE_SCHEMA, Userid};

#[api(
    input: {
        properties: {
            node: {
                schema: NODE_SCHEMA,
            },
        },
    },
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
                description: "The unique server ID, if permitted to access.",
            },
            url: {
                type: String,
                description: "URL to Web Shop.",
            },
        },
    },
    access: {
        permission: &Permission::Anybody,
    },
)]
/// Read subscription info.
fn get_subscription(
    _param: Value,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {
    let userid: Userid = rpcenv.get_user().unwrap().parse()?;
    let user_info = CachedUserInfo::new()?;
    let user_privs = user_info.lookup_privs(&userid, &[]);
    let server_id = if (user_privs & PRIV_SYS_AUDIT) != 0 {
        tools::get_hardware_address()?
    } else {
        "hidden".to_string()
    };

    let url = "https://www.proxmox.com/en/proxmox-backup-server/pricing";
    Ok(json!({
        "status": "NotFound",
        "message": "There is no subscription key",
        "serverid": server_id,
        "url":  url,
     }))
}

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_GET_SUBSCRIPTION);
