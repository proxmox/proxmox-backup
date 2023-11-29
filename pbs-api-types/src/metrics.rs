use serde::{Deserialize, Serialize};

use crate::{
    HOST_PORT_SCHEMA, HTTP_URL_SCHEMA, PROXMOX_SAFE_ID_FORMAT, SINGLE_LINE_COMMENT_SCHEMA,
};
use proxmox_schema::{api, Schema, StringSchema, Updater};

pub const METRIC_SERVER_ID_SCHEMA: Schema = StringSchema::new("Metrics Server ID.")
    .format(&PROXMOX_SAFE_ID_FORMAT)
    .min_length(3)
    .max_length(32)
    .schema();

pub const INFLUXDB_BUCKET_SCHEMA: Schema = StringSchema::new("InfluxDB Bucket.")
    .format(&PROXMOX_SAFE_ID_FORMAT)
    .min_length(3)
    .max_length(32)
    .default("proxmox")
    .schema();

pub const INFLUXDB_ORGANIZATION_SCHEMA: Schema = StringSchema::new("InfluxDB Organization.")
    .format(&PROXMOX_SAFE_ID_FORMAT)
    .min_length(3)
    .max_length(32)
    .default("proxmox")
    .schema();

fn return_true() -> bool {
    true
}

fn is_true(b: &bool) -> bool {
    *b
}

#[api(
    properties: {
        name: {
            schema: METRIC_SERVER_ID_SCHEMA,
        },
        enable: {
            type: bool,
            optional: true,
            default: true,
        },
        host: {
            schema: HOST_PORT_SCHEMA,
        },
        mtu: {
            type: u16,
            optional: true,
            default: 1500,
        },
        comment: {
            optional: true,
            schema: SINGLE_LINE_COMMENT_SCHEMA,
        },
    },
)]
#[derive(Serialize, Deserialize, Updater)]
#[serde(rename_all = "kebab-case")]
/// InfluxDB Server (UDP)
pub struct InfluxDbUdp {
    #[updater(skip)]
    pub name: String,
    #[serde(default = "return_true", skip_serializing_if = "is_true")]
    #[updater(serde(skip_serializing_if = "Option::is_none"))]
    /// Enables or disables the metrics server
    pub enable: bool,
    /// the host + port
    pub host: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// The MTU
    pub mtu: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

#[api(
    properties: {
        name: {
            schema: METRIC_SERVER_ID_SCHEMA,
        },
        enable: {
            type: bool,
            optional: true,
            default: true,
        },
        url: {
            schema: HTTP_URL_SCHEMA,
        },
        token: {
            type: String,
            optional: true,
        },
        bucket: {
            schema: INFLUXDB_BUCKET_SCHEMA,
            optional: true,
        },
        organization: {
            schema: INFLUXDB_ORGANIZATION_SCHEMA,
            optional: true,
        },
        "max-body-size": {
            type: usize,
            optional: true,
            default: 25_000_000,
        },
        "verify-tls": {
            type: bool,
            optional: true,
            default: true,
        },
        comment: {
            optional: true,
            schema: SINGLE_LINE_COMMENT_SCHEMA,
        },
    },
)]
#[derive(Serialize, Deserialize, Updater)]
#[serde(rename_all = "kebab-case")]
/// InfluxDB Server (HTTP(s))
pub struct InfluxDbHttp {
    #[updater(skip)]
    pub name: String,
    #[serde(default = "return_true", skip_serializing_if = "is_true")]
    #[updater(serde(skip_serializing_if = "Option::is_none"))]
    /// Enables or disables the metrics server
    pub enable: bool,
    /// The base url of the influxdb server
    pub url: String,
    /// The Optional Token
    #[serde(skip_serializing_if = "Option::is_none")]
    /// The (optional) API token
    pub token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bucket: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub organization: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// The (optional) maximum body size
    pub max_body_size: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// If true, the certificate will be validated.
    pub verify_tls: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

#[api]
#[derive(Copy, Clone, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
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
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
/// Basic information about a metric server that's available for all types
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
