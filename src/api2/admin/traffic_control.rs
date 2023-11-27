use anyhow::Error;

use proxmox_router::{Permission, Router, RpcEnvironment};
use proxmox_schema::api;

use pbs_api_types::{TrafficControlCurrentRate, TrafficControlRule, PRIV_SYS_AUDIT};

use crate::traffic_control_cache::TRAFFIC_CONTROL_CACHE;

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
    rpcenv: &mut dyn RpcEnvironment,
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
        list.push(TrafficControlCurrentRate {
            config,
            cur_rate_in,
            cur_rate_out,
        });
    }

    // also return the configuration digest
    rpcenv["digest"] = hex::encode(digest).into();

    Ok(list)
}

pub const ROUTER: Router = Router::new().get(&API_METHOD_SHOW_CURRENT_TRAFFIC);
