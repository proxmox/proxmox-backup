//! API Type Definitions

use anyhow::bail;
use serde::{Deserialize, Serialize};

use proxmox::api::{api, schema::*};

use pbs_api_types::StorageStatus;

mod acme;
pub use acme::*;

// File names: may not contain slashes, may not start with "."
pub const FILENAME_FORMAT: ApiStringFormat = ApiStringFormat::VerifyFn(|name| {
    if name.starts_with('.') {
        bail!("file names may not start with '.'");
    }
    if name.contains('/') {
        bail!("file names may not contain slashes");
    }
    Ok(())
});


// Complex type definitions


// Regression tests

#[test]
fn test_cert_fingerprint_schema() -> Result<(), anyhow::Error> {

    let schema = CERT_FINGERPRINT_SHA256_SCHEMA;

    let invalid_fingerprints = [
        "86:88:7c:be:26:77:a5:62:67:d9:06:f5:e4::61:3e:20:dc:cd:43:92:07:7f:fb:65:54:6c:ff:d2:96:36:f8",
        "88:7C:BE:26:77:a5:62:67:D9:06:f5:e4:14:61:3e:20:dc:cd:43:92:07:7f:fb:65:54:6c:ff:d2:96:36:f8",
        "86:88:7c:be:26:77:a5:62:67:d9:06:f5:e4::14:61:3e:20:dc:cd:43:92:07:7f:fb:65:54:6c:ff:d2:96:36:f8:ff",
        "XX:88:7c:be:26:77:a5:62:67:d9:06:f5:e4::14:61:3e:20:dc:cd:43:92:07:7f:fb:65:54:6c:ff:d2:96:36:f8",
        "86:88:Y4:be:26:77:a5:62:67:d9:06:f5:e4:14:61:3e:20:dc:cd:43:92:07:7f:fb:65:54:6c:ff:d2:96:36:f8",
        "86:88:0:be:26:77:a5:62:67:d9:06:f5:e4:14:61:3e:20:dc:cd:43:92:07:7f:fb:65:54:6c:ff:d2:96:36:f8",
    ];

    for fingerprint in invalid_fingerprints.iter() {
        if parse_simple_value(fingerprint, &schema).is_ok() {
            bail!("test fingerprint '{}' failed -  got Ok() while exception an error.", fingerprint);
        }
    }

    let valid_fingerprints = [
        "86:88:7c:be:26:77:a5:62:67:d9:06:f5:e4:14:61:3e:20:dc:cd:43:92:07:7f:fb:65:54:6c:ff:d2:96:36:f8",
        "86:88:7C:BE:26:77:a5:62:67:D9:06:f5:e4:14:61:3e:20:dc:cd:43:92:07:7f:fb:65:54:6c:ff:d2:96:36:f8",
    ];

    for fingerprint in valid_fingerprints.iter() {
        let v = match parse_simple_value(fingerprint, &schema) {
            Ok(v) => v,
            Err(err) => {
                bail!("unable to parse fingerprint '{}' - {}", fingerprint, err);
            }
        };

        if v != serde_json::json!(fingerprint) {
            bail!("unable to parse fingerprint '{}' - got wrong value {:?}", fingerprint, v);
        }
    }

    Ok(())
}

#[test]
fn test_proxmox_user_id_schema() -> Result<(), anyhow::Error> {
    let invalid_user_ids = [
        "x", // too short
        "xx", // too short
        "xxx", // no realm
        "xxx@", // no realm
        "xx x@test", // contains space
        "xx\nx@test", // contains control character
        "x:xx@test", // contains collon
        "xx/x@test", // contains slash
        "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx@test", // too long
    ];

    for name in invalid_user_ids.iter() {
        if parse_simple_value(name, &Userid::API_SCHEMA).is_ok() {
            bail!("test userid '{}' failed -  got Ok() while exception an error.", name);
        }
    }

    let valid_user_ids = [
        "xxx@y",
        "name@y",
        "xxx@test-it.com",
        "xxx@_T_E_S_T-it.com",
        "x_x-x.x@test-it.com",
    ];

    for name in valid_user_ids.iter() {
        let v = match parse_simple_value(name, &Userid::API_SCHEMA) {
            Ok(v) => v,
            Err(err) => {
                bail!("unable to parse userid '{}' - {}", name, err);
            }
        };

        if v != serde_json::json!(name) {
            bail!("unable to parse userid '{}' - got wrong value {:?}", name, v);
        }
    }

    Ok(())
}

#[api]
#[derive(Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
/// Node memory usage counters
pub struct NodeMemoryCounters {
    /// Total memory
    pub total: u64,
    /// Used memory
    pub used: u64,
    /// Free memory
    pub free: u64,
}

#[api]
#[derive(Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
/// Node swap usage counters
pub struct NodeSwapCounters {
    /// Total swap
    pub total: u64,
    /// Used swap
    pub used: u64,
    /// Free swap
    pub free: u64,
}

#[api]
#[derive(Serialize,Deserialize,Default)]
#[serde(rename_all = "kebab-case")]
/// Contains general node information such as the fingerprint`
pub struct NodeInformation {
    /// The SSL Fingerprint
    pub fingerprint: String,
}

#[api]
#[derive(Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
/// Information about the CPU
pub struct NodeCpuInformation {
    /// The CPU model
    pub model: String,
    /// The number of CPU sockets
    pub sockets: usize,
    /// The number of CPU cores (incl. threads)
    pub cpus: usize,
}

#[api(
    properties: {
        memory: {
            type: NodeMemoryCounters,
        },
        root: {
            type: StorageStatus,
        },
        swap: {
            type: NodeSwapCounters,
        },
        loadavg: {
            type: Array,
            items: {
                type: Number,
                description: "the load",
            }
        },
        cpuinfo: {
            type: NodeCpuInformation,
        },
        info: {
            type: NodeInformation,
        }
    },
)]
#[derive(Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
/// The Node status
pub struct NodeStatus {
    pub memory: NodeMemoryCounters,
    pub root: StorageStatus,
    pub swap: NodeSwapCounters,
    /// The current uptime of the server.
    pub uptime: u64,
    /// Load for 1, 5 and 15 minutes.
    pub loadavg: [f64; 3],
    /// The current kernel version.
    pub kversion: String,
    /// Total CPU usage since last query.
    pub cpu: f64,
    /// Total IO wait since last query.
    pub wait: f64,
    pub cpuinfo: NodeCpuInformation,
    pub info: NodeInformation,
}

pub const HTTP_PROXY_SCHEMA: Schema = StringSchema::new(
    "HTTP proxy configuration [http://]<host>[:port]")
    .format(&ApiStringFormat::VerifyFn(|s| {
        proxmox_http::ProxyConfig::parse_proxy_url(s)?;
        Ok(())
    }))
    .min_length(1)
    .max_length(128)
    .type_text("[http://]<host>[:port]")
    .schema();
