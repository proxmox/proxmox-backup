use serde::{Deserialize, Serialize};

use proxmox_human_byte::HumanByte;
use proxmox_schema::{api, IntegerSchema, Schema, StringSchema, Updater};

use crate::{
    CIDR_SCHEMA, DAILY_DURATION_FORMAT, PROXMOX_SAFE_ID_FORMAT, SINGLE_LINE_COMMENT_SCHEMA,
};

pub const TRAFFIC_CONTROL_TIMEFRAME_SCHEMA: Schema =
    StringSchema::new("Timeframe to specify when the rule is active.")
        .format(&DAILY_DURATION_FORMAT)
        .schema();

pub const TRAFFIC_CONTROL_ID_SCHEMA: Schema = StringSchema::new("Rule ID.")
    .format(&PROXMOX_SAFE_ID_FORMAT)
    .min_length(3)
    .max_length(32)
    .schema();

pub const TRAFFIC_CONTROL_RATE_SCHEMA: Schema =
    IntegerSchema::new("Rate limit (for Token bucket filter) in bytes/second.")
        .minimum(100_000)
        .schema();

pub const TRAFFIC_CONTROL_BURST_SCHEMA: Schema =
    IntegerSchema::new("Size of the token bucket (for Token bucket filter) in bytes.")
        .minimum(1000)
        .schema();

#[api(
    properties: {
        "rate-in": {
            type: HumanByte,
            optional: true,
        },
        "burst-in": {
            type: HumanByte,
            optional: true,
        },
        "rate-out": {
            type: HumanByte,
            optional: true,
        },
        "burst-out": {
            type: HumanByte,
            optional: true,
        },
    },
)]
#[derive(Serialize, Deserialize, Default, Clone, Updater, PartialEq)]
#[serde(rename_all = "kebab-case")]
///  Rate Limit Configuration
pub struct RateLimitConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_in: Option<HumanByte>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub burst_in: Option<HumanByte>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_out: Option<HumanByte>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub burst_out: Option<HumanByte>,
}

impl RateLimitConfig {
    pub fn with_same_inout(rate: Option<HumanByte>, burst: Option<HumanByte>) -> Self {
        Self {
            rate_in: rate,
            burst_in: burst,
            rate_out: rate,
            burst_out: burst,
        }
    }
}

#[api(
    properties: {
        name: {
            schema: TRAFFIC_CONTROL_ID_SCHEMA,
        },
        comment: {
            optional: true,
            schema: SINGLE_LINE_COMMENT_SCHEMA,
        },
        limit: {
            type: RateLimitConfig,
        },
        network: {
            type: Array,
            items: {
                schema: CIDR_SCHEMA,
            },
        },
        timeframe: {
            type: Array,
            items: {
                schema: TRAFFIC_CONTROL_TIMEFRAME_SCHEMA,
            },
            optional: true,
        },
    },
)]
#[derive(Clone, Serialize, Deserialize, PartialEq, Updater)]
#[serde(rename_all = "kebab-case")]
///  Traffic control rule
pub struct TrafficControlRule {
    #[updater(skip)]
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    /// Rule applies to Source IPs within this networks
    pub network: Vec<String>,
    #[serde(flatten)]
    pub limit: RateLimitConfig,
    // fixme: expose this?
    //    /// Bandwidth is shared across all connections
    //    #[serde(skip_serializing_if="Option::is_none")]
    //    pub shared: Option<bool>,
    /// Enable the rule at specific times
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeframe: Option<Vec<String>>,
}

#[api(
    properties: {
        config: {
            type: TrafficControlRule,
        },
    },
)]
#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
/// Traffic control rule config with current rates
pub struct TrafficControlCurrentRate {
    #[serde(flatten)]
    pub config: TrafficControlRule,
    /// Current ingress rate in bytes/second
    pub cur_rate_in: u64,
    /// Current egress rate in bytes/second
    pub cur_rate_out: u64,
}
