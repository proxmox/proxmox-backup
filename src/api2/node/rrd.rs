use anyhow::{bail, Error};
use serde_json::{json, Value};
use std::collections::BTreeMap;

use proxmox_router::{Permission, Router};
use proxmox_schema::api;

use pbs_api_types::{RRDMode, RRDTimeFrame, NODE_SCHEMA, PRIV_SYS_AUDIT};

use crate::rrd_cache::extract_rrd_data;

pub fn create_value_from_rrd(
    basedir: &str,
    list: &[&str],
    timeframe: RRDTimeFrame,
    mode: RRDMode,
) -> Result<Value, Error> {
    let mut result: Vec<Value> = Vec::new();

    let mut timemap = BTreeMap::new();

    let mut last_resolution = None;

    for name in list {
        let (start, reso, data) = match extract_rrd_data(basedir, name, timeframe, mode)? {
            Some(result) => result.into(),
            None => continue,
        };

        if let Some(expected_resolution) = last_resolution {
            if reso != expected_resolution {
                bail!(
                    "got unexpected RRD resolution ({} != {})",
                    reso,
                    expected_resolution
                );
            }
        } else {
            last_resolution = Some(reso);
        }

        let mut t = start;

        for value in data {
            let entry = timemap.entry(t).or_insert_with(|| json!({ "time": t }));
            if let Some(value) = value {
                entry[*name] = value.into();
            }
            t += reso;
        }
    }

    for item in timemap.values() {
        result.push(item.clone());
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
                type: RRDTimeFrame,
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
fn get_node_stats(timeframe: RRDTimeFrame, cf: RRDMode, _param: Value) -> Result<Value, Error> {
    create_value_from_rrd(
        "host",
        &[
            "cpu",
            "iowait",
            "memtotal",
            "memused",
            "swaptotal",
            "swapused",
            "netin",
            "netout",
            "loadavg",
            "total",
            "used",
            "read_ios",
            "read_bytes",
            "write_ios",
            "write_bytes",
            "io_ticks",
        ],
        timeframe,
        cf,
    )
}

pub const ROUTER: Router = Router::new().get(&API_METHOD_GET_NODE_STATS);
