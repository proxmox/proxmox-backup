//! API Type Definitions

use anyhow::bail;
use serde::{Deserialize, Serialize};

use proxmox::api::{api, schema::*};
use proxmox::const_regex;

use crate::config::acl::Role;

mod acme;
pub use acme::*;

pub use pbs_api_types::*;

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

const_regex!{
    pub SYSTEMD_DATETIME_REGEX = r"^\d{4}-\d{2}-\d{2}( \d{2}:\d{2}(:\d{2})?)?$"; //  fixme: define in common_regex ?

    pub ACL_PATH_REGEX = concat!(r"^(?:/|", r"(?:/", PROXMOX_SAFE_ID_REGEX_STR!(), ")+", r")$");

    pub SUBSCRIPTION_KEY_REGEX = concat!(r"^pbs(?:[cbsp])-[0-9a-f]{10}$");

    pub ZPOOL_NAME_REGEX = r"^[a-zA-Z][a-z0-9A-Z\-_.:]+$";

    pub DATASTORE_MAP_REGEX = concat!(r"(:?", PROXMOX_SAFE_ID_REGEX_STR!(), r"=)?", PROXMOX_SAFE_ID_REGEX_STR!());

}

pub const SYSTEMD_DATETIME_FORMAT: ApiStringFormat =
    ApiStringFormat::Pattern(&SYSTEMD_DATETIME_REGEX);

pub const HOSTNAME_FORMAT: ApiStringFormat =
    ApiStringFormat::Pattern(&HOSTNAME_REGEX);

pub const DNS_ALIAS_FORMAT: ApiStringFormat =
    ApiStringFormat::Pattern(&DNS_ALIAS_REGEX);

pub const ACL_PATH_FORMAT: ApiStringFormat =
    ApiStringFormat::Pattern(&ACL_PATH_REGEX);


pub const SUBSCRIPTION_KEY_FORMAT: ApiStringFormat =
    ApiStringFormat::Pattern(&SUBSCRIPTION_KEY_REGEX);

pub const BLOCKDEVICE_NAME_FORMAT: ApiStringFormat =
    ApiStringFormat::Pattern(&BLOCKDEVICE_NAME_REGEX);

pub const DATASTORE_MAP_FORMAT: ApiStringFormat =
    ApiStringFormat::Pattern(&DATASTORE_MAP_REGEX);

pub const PASSWORD_SCHEMA: Schema = StringSchema::new("Password.")
    .format(&PASSWORD_FORMAT)
    .min_length(1)
    .max_length(1024)
    .schema();

pub const PBS_PASSWORD_SCHEMA: Schema = StringSchema::new("User Password.")
    .format(&PASSWORD_FORMAT)
    .min_length(5)
    .max_length(64)
    .schema();

pub const CHUNK_DIGEST_SCHEMA: Schema = StringSchema::new("Chunk digest (SHA256).")
    .format(&CHUNK_DIGEST_FORMAT)
    .schema();

pub const NODE_SCHEMA: Schema = StringSchema::new("Node name (or 'localhost')")
    .format(&ApiStringFormat::VerifyFn(|node| {
        if node == "localhost" || node == proxmox::tools::nodename() {
            Ok(())
        } else {
            bail!("no such node '{}'", node);
        }
    }))
    .schema();

pub const SEARCH_DOMAIN_SCHEMA: Schema =
    StringSchema::new("Search domain for host-name lookup.").schema();

pub const FIRST_DNS_SERVER_SCHEMA: Schema =
    StringSchema::new("First name server IP address.")
    .format(&IP_FORMAT)
    .schema();

pub const SECOND_DNS_SERVER_SCHEMA: Schema =
    StringSchema::new("Second name server IP address.")
    .format(&IP_FORMAT)
    .schema();

pub const THIRD_DNS_SERVER_SCHEMA: Schema =
    StringSchema::new("Third name server IP address.")
    .format(&IP_FORMAT)
    .schema();


pub const TIME_ZONE_SCHEMA: Schema = StringSchema::new(
    "Time zone. The file '/usr/share/zoneinfo/zone.tab' contains the list of valid names.")
    .format(&SINGLE_LINE_COMMENT_FORMAT)
    .min_length(2)
    .max_length(64)
    .schema();

pub const ACL_PATH_SCHEMA: Schema = StringSchema::new(
    "Access control path.")
    .format(&ACL_PATH_FORMAT)
    .min_length(1)
    .max_length(128)
    .schema();

pub const ACL_PROPAGATE_SCHEMA: Schema = BooleanSchema::new(
    "Allow to propagate (inherit) permissions.")
    .default(true)
    .schema();

pub const ACL_UGID_TYPE_SCHEMA: Schema = StringSchema::new(
    "Type of 'ugid' property.")
    .format(&ApiStringFormat::Enum(&[
        EnumEntry::new("user", "User"),
        EnumEntry::new("group", "Group")]))
    .schema();

#[api(
    properties: {
        propagate: {
            schema: ACL_PROPAGATE_SCHEMA,
        },
	path: {
            schema: ACL_PATH_SCHEMA,
        },
        ugid_type: {
            schema: ACL_UGID_TYPE_SCHEMA,
        },
	ugid: {
            type: String,
            description: "User or Group ID.",
        },
	roleid: {
            type: Role,
        }
    }
)]
#[derive(Serialize, Deserialize)]
/// ACL list entry.
pub struct AclListItem {
    pub path: String,
    pub ugid: String,
    pub ugid_type: String,
    pub propagate: bool,
    pub roleid: String,
}

pub const DATASTORE_MAP_SCHEMA: Schema = StringSchema::new("Datastore mapping.")
    .format(&DATASTORE_MAP_FORMAT)
    .min_length(3)
    .max_length(65)
    .type_text("(<source>=)?<target>")
    .schema();

pub const DATASTORE_MAP_ARRAY_SCHEMA: Schema = ArraySchema::new(
    "Datastore mapping list.", &DATASTORE_MAP_SCHEMA)
    .schema();

pub const DATASTORE_MAP_LIST_SCHEMA: Schema = StringSchema::new(
    "A list of Datastore mappings (or single datastore), comma separated. \
    For example 'a=b,e' maps the source datastore 'a' to target 'b and \
    all other sources to the default 'e'. If no default is given, only the \
    specified sources are mapped.")
    .format(&ApiStringFormat::PropertyString(&DATASTORE_MAP_ARRAY_SCHEMA))
    .schema();


pub const HOSTNAME_SCHEMA: Schema = StringSchema::new("Hostname (as defined in RFC1123).")
    .format(&HOSTNAME_FORMAT)
    .schema();

pub const SUBSCRIPTION_KEY_SCHEMA: Schema = StringSchema::new("Proxmox Backup Server subscription key.")
    .format(&SUBSCRIPTION_KEY_FORMAT)
    .min_length(15)
    .max_length(16)
    .schema();

pub const BLOCKDEVICE_NAME_SCHEMA: Schema = StringSchema::new("Block device name (/sys/block/<name>).")
    .format(&BLOCKDEVICE_NAME_FORMAT)
    .min_length(3)
    .max_length(64)
    .schema();

// Complex type definitions

#[api(
    properties: {
        "gc-status": {
            type: GarbageCollectionStatus,
            optional: true,
        },
        counts: {
            type: Counts,
            optional: true,
        },
    },
)]
#[derive(Serialize, Deserialize)]
#[serde(rename_all="kebab-case")]
/// Overall Datastore status and useful information.
pub struct DataStoreStatus {
    /// Total space (bytes).
    pub total: u64,
    /// Used space (bytes).
    pub used: u64,
    /// Available space (bytes).
    pub avail: u64,
    /// Status of last GC
    #[serde(skip_serializing_if="Option::is_none")]
    pub gc_status: Option<GarbageCollectionStatus>,
    /// Group/Snapshot counts
    #[serde(skip_serializing_if="Option::is_none")]
    pub counts: Option<Counts>,
}

#[api()]
#[derive(Eq, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskStateType {
    /// Ok
    OK,
    /// Warning
    Warning,
    /// Error
    Error,
    /// Unknown
    Unknown,
}

#[api()]
#[derive(Debug, Copy, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
/// Node Power command type.
pub enum NodePowerCommand {
    /// Restart the server
    Reboot,
    /// Shutdown the server
    Shutdown,
}

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

#[api()]
#[derive(Copy, Clone, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum RRDMode {
    /// Maximum
    Max,
    /// Average
    Average,
}


#[api()]
#[repr(u64)]
#[derive(Copy, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RRDTimeFrameResolution {
    ///  1 min => last 70 minutes
    Hour = 60,
    /// 30 min => last 35 hours
    Day = 60*30,
    /// 3 hours => about 8 days
    Week = 60*180,
    /// 12 hours => last 35 days
    Month = 60*720,
    /// 1 week => last 490 days
    Year = 60*10080,
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
