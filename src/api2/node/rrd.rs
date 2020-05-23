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
/// Read CPU stats
fn get_cpu_stats(
    timeframe: RRDTimeFrameResolution,
    cf: RRDMode,
    _param: Value,
) -> Result<Value, Error> {

    crate::rrd::extract_data("host/cpu", timeframe, cf)
}

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_GET_CPU_STATS);
