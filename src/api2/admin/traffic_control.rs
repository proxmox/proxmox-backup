use anyhow::Error;
use serde::{Deserialize, Serialize};

use proxmox_router::{Router, RpcEnvironment, Permission};
use proxmox_schema::api;

use pbs_api_types::{
    TrafficControlRule, PRIV_SYS_AUDIT,
};

use crate::traffic_control_cache::TRAFFIC_CONTROL_CACHE;

#[api(
    properties: {
        config: {
            type: TrafficControlRule,
        },
    },
)]
#[derive(Serialize, Deserialize)]
#[serde(rename_all="kebab-case")]
/// Traffic control rule config with current rates
pub struct TrafficControlCurrentRate {
    #[serde(flatten)]
    config: TrafficControlRule,
    /// Current ingress rate in bytes/second
    cur_rate_in: u64,
    /// Current egress rate in bytes/second
    cur_rate_out: u64,
}

#[api(
    input: {
        properties: {},
    },
    returns: {
        description: "Show current traffic control rates.",
        type: Array,
        items: {
            type: TrafficControlCurrentRate,
        },
    },
    access: {
        permission: &Permission::Privilege(&[], PRIV_SYS_AUDIT, false),
    },
)]
/// Show current traffic for all traffic control rules.
pub fn show_current_traffic(
    mut rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<TrafficControlCurrentRate>, Error> {

    let (config, digest) = pbs_config::traffic_control::config()?;

    let rules: Vec<TrafficControlRule> = config.convert_to_typed_array("rule")?;

    let cache = TRAFFIC_CONTROL_CACHE.lock().unwrap();

    let mut list = Vec::new();

    for config in rules {
        let (cur_rate_in, cur_rate_out) = match cache.current_rate_map().get(&config.name) {
            None => (0, 0),
            Some(state) => (state.rate_in, state.rate_out),
        };
        list.push(TrafficControlCurrentRate {config, cur_rate_in, cur_rate_out});
    }

    // also return the configuration digest
    rpcenv["digest"] = hex::encode(&digest).into();

    Ok(list)
}

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_SHOW_CURRENT_TRAFFIC);
