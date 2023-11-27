use anyhow::Error;
use serde_json::Value;

use proxmox_router::{Permission, Router, RpcEnvironment};
use proxmox_schema::api;

use pbs_api_types::{MetricServerInfo, PRIV_SYS_AUDIT};
use pbs_config::metrics;

#[api(
    input: {
        properties: {},
    },
    returns: {
        description: "List of configured metric servers.",
        type: Array,
        items: { type: MetricServerInfo },
    },
    access: {
        permission: &Permission::Privilege(&[], PRIV_SYS_AUDIT, false),
    },
)]
/// List configured metric servers.
pub fn list_metric_servers(
    _param: Value,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<MetricServerInfo>, Error> {
    let (config, digest) = metrics::config()?;
    let mut list = Vec::new();

    for (_, (section_type, v)) in config.sections.iter() {
        let mut entry = v.clone();
        entry["type"] = Value::from(section_type.clone());
        if entry.get("url").is_some() {
            entry["server"] = entry["url"].clone();
        }
        if entry.get("host").is_some() {
            entry["server"] = entry["host"].clone();
        }
        list.push(serde_json::from_value(entry)?);
    }

    rpcenv["digest"] = hex::encode(digest).into();

    Ok(list)
}

pub const ROUTER: Router = Router::new().get(&API_METHOD_LIST_METRIC_SERVERS);
