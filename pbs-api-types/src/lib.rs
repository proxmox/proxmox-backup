//! Basic API types used by most of the PBS code.

use const_format::concatcp;
use serde::{Deserialize, Serialize};

pub mod percent_encoding;

use proxmox_schema::{
    api, const_regex, ApiStringFormat, ApiType, ArraySchema, ReturnType, Schema, StringSchema,
};
use proxmox_time::parse_daily_duration;

use proxmox_auth_api::types::{APITOKEN_ID_REGEX_STR, USER_ID_REGEX_STR};

pub use proxmox_schema::api_types::SAFE_ID_FORMAT as PROXMOX_SAFE_ID_FORMAT;
pub use proxmox_schema::api_types::SAFE_ID_REGEX as PROXMOX_SAFE_ID_REGEX;
pub use proxmox_schema::api_types::SAFE_ID_REGEX_STR as PROXMOX_SAFE_ID_REGEX_STR;
pub use proxmox_schema::api_types::{
    BLOCKDEVICE_DISK_AND_PARTITION_NAME_REGEX, BLOCKDEVICE_NAME_REGEX,
};
pub use proxmox_schema::api_types::{DNS_ALIAS_REGEX, DNS_NAME_OR_IP_REGEX, DNS_NAME_REGEX};
pub use proxmox_schema::api_types::{FINGERPRINT_SHA256_REGEX, SHA256_HEX_REGEX};
pub use proxmox_schema::api_types::{
    GENERIC_URI_REGEX, HOSTNAME_REGEX, HOST_PORT_REGEX, HTTP_URL_REGEX,
};
pub use proxmox_schema::api_types::{MULTI_LINE_COMMENT_REGEX, SINGLE_LINE_COMMENT_REGEX};
pub use proxmox_schema::api_types::{PASSWORD_REGEX, SYSTEMD_DATETIME_REGEX, UUID_REGEX};

pub use proxmox_schema::api_types::{CIDR_FORMAT, CIDR_REGEX};
pub use proxmox_schema::api_types::{CIDR_V4_FORMAT, CIDR_V4_REGEX};
pub use proxmox_schema::api_types::{CIDR_V6_FORMAT, CIDR_V6_REGEX};
pub use proxmox_schema::api_types::{IPRE_STR, IP_FORMAT, IP_REGEX};
pub use proxmox_schema::api_types::{IPV4RE_STR, IP_V4_FORMAT, IP_V4_REGEX};
pub use proxmox_schema::api_types::{IPV6RE_STR, IP_V6_FORMAT, IP_V6_REGEX};

pub use proxmox_schema::api_types::COMMENT_SCHEMA as SINGLE_LINE_COMMENT_SCHEMA;
pub use proxmox_schema::api_types::HOSTNAME_SCHEMA;
pub use proxmox_schema::api_types::HOST_PORT_SCHEMA;
pub use proxmox_schema::api_types::HTTP_URL_SCHEMA;
pub use proxmox_schema::api_types::MULTI_LINE_COMMENT_SCHEMA;
pub use proxmox_schema::api_types::NODE_SCHEMA;
pub use proxmox_schema::api_types::SINGLE_LINE_COMMENT_FORMAT;
pub use proxmox_schema::api_types::{
    BLOCKDEVICE_DISK_AND_PARTITION_NAME_SCHEMA, BLOCKDEVICE_NAME_SCHEMA,
};
pub use proxmox_schema::api_types::{CERT_FINGERPRINT_SHA256_SCHEMA, FINGERPRINT_SHA256_FORMAT};
pub use proxmox_schema::api_types::{DISK_ARRAY_SCHEMA, DISK_LIST_SCHEMA};
pub use proxmox_schema::api_types::{DNS_ALIAS_FORMAT, DNS_NAME_FORMAT, DNS_NAME_OR_IP_SCHEMA};
pub use proxmox_schema::api_types::{PASSWORD_FORMAT, PASSWORD_SCHEMA};
pub use proxmox_schema::api_types::{SERVICE_ID_SCHEMA, UUID_FORMAT};
pub use proxmox_schema::api_types::{SYSTEMD_DATETIME_FORMAT, TIME_ZONE_SCHEMA};

use proxmox_schema::api_types::{DNS_NAME_STR, IPRE_BRACKET_STR};

#[rustfmt::skip]
pub const BACKUP_ID_RE: &str = r"[A-Za-z0-9_][A-Za-z0-9._\-]*";

#[rustfmt::skip]
pub const BACKUP_TYPE_RE: &str = r"(?:host|vm|ct)";

#[rustfmt::skip]
pub const BACKUP_TIME_RE: &str = r"[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z";

#[rustfmt::skip]
pub const BACKUP_NS_RE: &str =
    concatcp!("(?:",
        "(?:", PROXMOX_SAFE_ID_REGEX_STR, r"/){0,7}", PROXMOX_SAFE_ID_REGEX_STR,
    ")?");

#[rustfmt::skip]
pub const BACKUP_NS_PATH_RE: &str =
    concatcp!(r"(?:ns/", PROXMOX_SAFE_ID_REGEX_STR, r"/){0,7}ns/", PROXMOX_SAFE_ID_REGEX_STR, r"/");

#[rustfmt::skip]
pub const SNAPSHOT_PATH_REGEX_STR: &str =
    concatcp!(
        r"(", BACKUP_TYPE_RE, ")/(", BACKUP_ID_RE, ")/(", BACKUP_TIME_RE, r")",
    );

#[rustfmt::skip]
pub const GROUP_OR_SNAPSHOT_PATH_REGEX_STR: &str =
    concatcp!(
        r"(", BACKUP_TYPE_RE, ")/(", BACKUP_ID_RE, ")(?:/(", BACKUP_TIME_RE, r"))?",
    );

mod acl;
pub use acl::*;

mod datastore;
pub use datastore::*;

mod jobs;
pub use jobs::*;

mod key_derivation;
pub use key_derivation::{Kdf, KeyInfo};

mod maintenance;
pub use maintenance::*;

mod network;
pub use network::*;

mod node;
pub use node::*;

pub use proxmox_auth_api::types as userid;
pub use proxmox_auth_api::types::{Authid, Userid};
pub use proxmox_auth_api::types::{Realm, RealmRef};
pub use proxmox_auth_api::types::{Tokenname, TokennameRef};
pub use proxmox_auth_api::types::{Username, UsernameRef};
pub use proxmox_auth_api::types::{
    PROXMOX_GROUP_ID_SCHEMA, PROXMOX_TOKEN_ID_SCHEMA, PROXMOX_TOKEN_NAME_SCHEMA,
};

#[macro_use]
mod user;
pub use user::*;

pub use proxmox_schema::upid::*;

mod crypto;
pub use crypto::{bytes_as_fingerprint, CryptMode, Fingerprint};

pub mod file_restore;

mod openid;
pub use openid::*;

mod ldap;
pub use ldap::*;

mod ad;
pub use ad::*;

mod remote;
pub use remote::*;

mod tape;
pub use tape::*;

mod traffic_control;
pub use traffic_control::*;

mod zfs;
pub use zfs::*;

mod metrics;
pub use metrics::*;

const_regex! {
    // just a rough check - dummy acceptor is used before persisting
    pub OPENSSL_CIPHERS_REGEX = r"^[0-9A-Za-z_:, +!\-@=.]+$";

    pub BACKUP_REPO_URL_REGEX = concatcp!(
        r"^^(?:(?:(",
        USER_ID_REGEX_STR, "|", APITOKEN_ID_REGEX_STR,
        ")@)?(",
        DNS_NAME_STR, "|",  IPRE_BRACKET_STR,
        "):)?(?:([0-9]{1,5}):)?(", PROXMOX_SAFE_ID_REGEX_STR, r")$"
    );

     pub SUBSCRIPTION_KEY_REGEX = concat!(r"^pbs(?:[cbsp])-[0-9a-f]{10}$");
}

pub const PVE_CONFIG_DIGEST_FORMAT: ApiStringFormat = ApiStringFormat::Pattern(&SHA256_HEX_REGEX);

pub const SUBSCRIPTION_KEY_FORMAT: ApiStringFormat =
    ApiStringFormat::Pattern(&SUBSCRIPTION_KEY_REGEX);

pub const OPENSSL_CIPHERS_TLS_FORMAT: ApiStringFormat =
    ApiStringFormat::Pattern(&OPENSSL_CIPHERS_REGEX);

pub const DAILY_DURATION_FORMAT: ApiStringFormat =
    ApiStringFormat::VerifyFn(|s| parse_daily_duration(s).map(drop));

pub const SEARCH_DOMAIN_SCHEMA: Schema =
    StringSchema::new("Search domain for host-name lookup.").schema();

pub const FIRST_DNS_SERVER_SCHEMA: Schema = StringSchema::new("First name server IP address.")
    .format(&IP_FORMAT)
    .schema();

pub const SECOND_DNS_SERVER_SCHEMA: Schema = StringSchema::new("Second name server IP address.")
    .format(&IP_FORMAT)
    .schema();

pub const THIRD_DNS_SERVER_SCHEMA: Schema = StringSchema::new("Third name server IP address.")
    .format(&IP_FORMAT)
    .schema();

pub const OPENSSL_CIPHERS_TLS_1_2_SCHEMA: Schema =
    StringSchema::new("OpenSSL cipher list used by the proxy for TLS <= 1.2")
        .format(&OPENSSL_CIPHERS_TLS_FORMAT)
        .schema();

pub const OPENSSL_CIPHERS_TLS_1_3_SCHEMA: Schema =
    StringSchema::new("OpenSSL ciphersuites list used by the proxy for TLS 1.3")
        .format(&OPENSSL_CIPHERS_TLS_FORMAT)
        .schema();

pub const PBS_PASSWORD_SCHEMA: Schema = StringSchema::new("User Password.")
    .format(&PASSWORD_FORMAT)
    .min_length(5)
    .max_length(64)
    .schema();

pub const REALM_ID_SCHEMA: Schema = StringSchema::new("Realm name.")
    .format(&PROXMOX_SAFE_ID_FORMAT)
    .min_length(2)
    .max_length(32)
    .schema();

pub const SUBSCRIPTION_KEY_SCHEMA: Schema =
    StringSchema::new("Proxmox Backup Server subscription key.")
        .format(&SUBSCRIPTION_KEY_FORMAT)
        .min_length(15)
        .max_length(16)
        .schema();

pub const PROXMOX_CONFIG_DIGEST_SCHEMA: Schema = StringSchema::new(
    "Prevent changes if current configuration file has different \
    SHA256 digest. This can be used to prevent concurrent \
    modifications.",
)
.format(&PVE_CONFIG_DIGEST_FORMAT)
.schema();

/// API schema format definition for repository URLs
pub const BACKUP_REPO_URL: ApiStringFormat = ApiStringFormat::Pattern(&BACKUP_REPO_URL_REGEX);

// Complex type definitions

#[api()]
#[derive(Default, Serialize, Deserialize)]
/// Storage space usage information.
pub struct StorageStatus {
    /// Total space (bytes).
    pub total: u64,
    /// Used space (bytes).
    pub used: u64,
    /// Available space (bytes).
    pub avail: u64,
}

pub const PASSWORD_HINT_SCHEMA: Schema = StringSchema::new("Password hint.")
    .format(&SINGLE_LINE_COMMENT_FORMAT)
    .min_length(1)
    .max_length(64)
    .schema();

#[api()]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
/// Describes a package for which an update is available.
pub struct APTUpdateInfo {
    /// Package name
    pub package: String,
    /// Package title
    pub title: String,
    /// Package architecture
    pub arch: String,
    /// Human readable package description
    pub description: String,
    /// New version to be updated to
    pub version: String,
    /// Old version currently installed
    pub old_version: String,
    /// Package origin
    pub origin: String,
    /// Package priority in human-readable form
    pub priority: String,
    /// Package section
    pub section: String,
    /// Custom extra field for additional package information
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_info: Option<String>,
}

#[api()]
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
/// Node Power command type.
pub enum NodePowerCommand {
    /// Restart the server
    Reboot,
    /// Shutdown the server
    Shutdown,
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

#[api(
    properties: {
        upid: { schema: UPID::API_SCHEMA },
    },
)]
#[derive(Serialize, Deserialize, Clone, PartialEq)]
/// Task properties.
pub struct TaskListItem {
    pub upid: String,
    /// The node name where the task is running on.
    pub node: String,
    /// The Unix PID
    pub pid: i64,
    /// The task start time (Epoch)
    pub pstart: u64,
    /// The task start time (Epoch)
    pub starttime: i64,
    /// Worker type (arbitrary ASCII string)
    pub worker_type: String,
    /// Worker ID (arbitrary ASCII string)
    pub worker_id: Option<String>,
    /// The authenticated entity who started the task
    pub user: String,
    /// The task end time (Epoch)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endtime: Option<i64>,
    /// Task end status
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

pub const NODE_TASKS_LIST_TASKS_RETURN_TYPE: ReturnType = ReturnType {
    optional: false,
    schema: &ArraySchema::new("A list of tasks.", &TaskListItem::API_SCHEMA).schema(),
};

#[api()]
#[derive(Copy, Clone, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
/// RRD consolidation mode
pub enum RRDMode {
    /// Maximum
    Max,
    /// Average
    Average,
}

#[api()]
#[derive(Copy, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
/// RRD time frame
pub enum RRDTimeFrame {
    /// Hour
    Hour,
    /// Day
    Day,
    /// Week
    Week,
    /// Month
    Month,
    /// Year
    Year,
    /// Decade (10 years)
    Decade,
}

#[api]
#[derive(Deserialize, Serialize, Copy, Clone, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
/// type of the realm
pub enum RealmType {
    /// The PAM realm
    Pam,
    /// The PBS realm
    Pbs,
    /// An OpenID Connect realm
    OpenId,
    /// An LDAP realm
    Ldap,
    /// An Active Directory (AD) realm
    Ad,
}

serde_plain::derive_display_from_serialize!(RealmType);
serde_plain::derive_fromstr_from_deserialize!(RealmType);

#[api(
    properties: {
        realm: {
            schema: REALM_ID_SCHEMA,
        },
        "type": {
            type: RealmType,
        },
        comment: {
            optional: true,
            schema: SINGLE_LINE_COMMENT_SCHEMA,
        },
    },
)]
#[derive(Deserialize, Serialize, Clone, PartialEq)]
#[serde(rename_all = "kebab-case")]
/// Basic Information about a realm
pub struct BasicRealmInfo {
    pub realm: String,
    #[serde(rename = "type")]
    pub ty: RealmType,
    /// True if it is the default realm
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}
