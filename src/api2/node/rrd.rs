use anyhow::Error;
use serde_json::Value;

use proxmox::api::{api, Router};

use crate::api2::types::*;

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
)]
/// Read node stats
fn get_node_stats(
    timeframe: RRDTimeFrameResolution,
    cf: RRDMode,
    _param: Value,
) -> Result<Value, Error> {

    crate::rrd::extract_data(
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
