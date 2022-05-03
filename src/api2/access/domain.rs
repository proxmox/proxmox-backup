//! List Authentication domains/realms

use anyhow::Error;
use serde_json::{json, Value};

use proxmox_router::{Permission, Router, RpcEnvironment};
use proxmox_schema::api;

use pbs_api_types::BasicRealmInfo;

#[api(
    returns: {
        description: "List of realms with basic info.",
        type: Array,
        items: {
            type: BasicRealmInfo,
        }
    },
    access: {
        description: "Anyone can access this, because we need that list for the login box (before the user is authenticated).",
        permission: &Permission::World,
    }
)]
/// Authentication domain/realm index.
fn list_domains(rpcenv: &mut dyn RpcEnvironment) -> Result<Vec<BasicRealmInfo>, Error> {
    let mut list = Vec::new();

    list.push(serde_json::from_value(json!({
        "realm": "pam",
        "type": "pam",
        "comment": "Linux PAM standard authentication",
        "default": Some(true),
    }))?);
    list.push(serde_json::from_value(json!({
        "realm": "pbs",
        "type": "pbs",
        "comment": "Proxmox Backup authentication server",
    }))?);

    let (config, digest) = pbs_config::domains::config()?;

    for (_, (section_type, v)) in config.sections.iter() {
        let mut entry = v.clone();
        entry["type"] = Value::from(section_type.clone());
        list.push(serde_json::from_value(entry)?);
    }

    rpcenv["digest"] = hex::encode(&digest).into();

    Ok(list)
}

pub const ROUTER: Router = Router::new().get(&API_METHOD_LIST_DOMAINS);
