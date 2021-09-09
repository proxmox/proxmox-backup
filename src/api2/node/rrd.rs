use anyhow::Error;
use serde_json::{Value, json};

use proxmox::api::{api, Permission, Router};

use pbs_api_types::{RRDMode, RRDTimeFrameResolution, NODE_SCHEMA, PRIV_SYS_AUDIT};

use crate::rrd::{extract_cached_data, RRD_DATA_ENTRIES};

pub fn create_value_from_rrd(
    basedir: &str,
    list: &[&str],
    timeframe: RRDTimeFrameResolution,
    cf: RRDMode,
) -> Result<Value, Error> {

    let mut result = Vec::new();
    let now = proxmox::tools::time::epoch_f64();

    for name in list {
        let (start, reso, list) = match extract_cached_data(basedir, name, now, timeframe, cf) {
            Some(result) => result,
            None => continue,
        };

        let mut t = start;
        for index in 0..RRD_DATA_ENTRIES {
            if result.len() <= index {
                if let Some(value) = list[index] {
                    result.push(json!({ "time": t, *name: value }));
                } else {
                    result.push(json!({ "time": t }));
                }
            } else if let Some(value) = list[index] {
                result[index][name] = value.into();
            }
            t += reso;
        }
    }

    Ok(result.into())
}

#[api(
    input: {
        properties: {
            node: {
                schema: NODE_SCHEMA,
            },
            timeframe: {
                type: RRDTimeFrameResolution,
            },
            cf: {
                type: RRDMode,
            },
        },
    },
    access: {
        permission: &Permission::Privilege(&["system", "status"], PRIV_SYS_AUDIT, false),
    },
)]
/// Read node stats
fn get_node_stats(
    timeframe: RRDTimeFrameResolution,
    cf: RRDMode,
    _param: Value,
) -> Result<Value, Error> {

    create_value_from_rrd(
        "host",
        &[
            "cpu", "iowait",
            "memtotal", "memused",
            "swaptotal", "swapused",
            "netin", "netout",
            "loadavg",
            "total", "used",
            "read_ios", "read_bytes",
            "write_ios", "write_bytes",
            "io_ticks",
         ],
        timeframe,
        cf,
    )
}

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_GET_NODE_STATS);
