use anyhow::Error;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use proxmox_router::{Permission, Router, RpcEnvironment};
use proxmox_schema::api;

use pbs_api_types::{METRIC_SERVER_ID_SCHEMA, PRIV_SYS_AUDIT, SINGLE_LINE_COMMENT_SCHEMA};
use pbs_config::metrics;

#[api]
#[derive(Deserialize, Serialize, PartialEq, Eq)]
/// Type of the metric server
pub enum MetricServerType {
    /// InfluxDB HTTP
    #[serde(rename = "influxdb-http")]
    InfluxDbHttp,
    /// InfluxDB UDP
    #[serde(rename = "influxdb-udp")]
    InfluxDbUdp,
}

#[api(
    properties: {
        name: {
            schema: METRIC_SERVER_ID_SCHEMA,
        },
        "type": {
            type: MetricServerType,
        },
        comment: {
            optional: true,
            schema: SINGLE_LINE_COMMENT_SCHEMA,
        },
    },
)]
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
/// Basic information about a metric server thats available for all types
pub struct MetricServerInfo {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: MetricServerType,
    /// Enables or disables the metrics server
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enable: Option<bool>,
    /// The target server
    pub server: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

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

    rpcenv["digest"] = hex::encode(&digest).into();

    Ok(list)
}

pub const ROUTER: Router = Router::new().get(&API_METHOD_LIST_METRIC_SERVERS);
