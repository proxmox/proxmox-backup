//! Basic API types used by most of the PBS code.

use serde::{Deserialize, Serialize};

use proxmox::api::api;
use proxmox::api::schema::{ApiStringFormat, Schema, StringSchema};
use proxmox::const_regex;
use proxmox::{IPRE, IPRE_BRACKET, IPV4OCTET, IPV4RE, IPV6H16, IPV6LS32, IPV6RE};

#[rustfmt::skip]
#[macro_export]
macro_rules! PROXMOX_SAFE_ID_REGEX_STR { () => { r"(?:[A-Za-z0-9_][A-Za-z0-9._\-]*)" }; }

#[rustfmt::skip]
#[macro_export]
macro_rules! BACKUP_ID_RE { () => (r"[A-Za-z0-9_][A-Za-z0-9._\-]*") }

#[rustfmt::skip]
#[macro_export]
macro_rules! BACKUP_TYPE_RE { () => (r"(?:host|vm|ct)") }

#[rustfmt::skip]
#[macro_export]
macro_rules! BACKUP_TIME_RE { () => (r"[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z") }

#[rustfmt::skip]
#[macro_export]
macro_rules! SNAPSHOT_PATH_REGEX_STR {
    () => (
        concat!(r"(", BACKUP_TYPE_RE!(), ")/(", BACKUP_ID_RE!(), ")/(", BACKUP_TIME_RE!(), r")")
    );
}

#[macro_use]
mod userid;
pub use userid::Authid;
pub use userid::Userid;
pub use userid::{Realm, RealmRef};
pub use userid::{Tokenname, TokennameRef};
pub use userid::{Username, UsernameRef};
pub use userid::{PROXMOX_GROUP_ID_SCHEMA, PROXMOX_TOKEN_ID_SCHEMA, PROXMOX_TOKEN_NAME_SCHEMA};

pub mod upid;
pub use upid::UPID;

#[rustfmt::skip]
#[macro_use]
mod local_macros {
    macro_rules! DNS_LABEL { () => (r"(?:[a-zA-Z0-9](?:[a-zA-Z0-9\-]*[a-zA-Z0-9])?)") }
    macro_rules! DNS_NAME { () => (concat!(r"(?:(?:", DNS_LABEL!() , r"\.)*", DNS_LABEL!(), ")")) }
    macro_rules! CIDR_V4_REGEX_STR { () => (concat!(r"(?:", IPV4RE!(), r"/\d{1,2})$")) }
    macro_rules! CIDR_V6_REGEX_STR { () => (concat!(r"(?:", IPV6RE!(), r"/\d{1,3})$")) }
    macro_rules! DNS_ALIAS_LABEL { () => (r"(?:[a-zA-Z0-9_](?:[a-zA-Z0-9\-]*[a-zA-Z0-9])?)") }
    macro_rules! DNS_ALIAS_NAME {
        () => (concat!(r"(?:(?:", DNS_ALIAS_LABEL!() , r"\.)*", DNS_ALIAS_LABEL!(), ")"))
    }
}

const_regex! {
    pub IP_V4_REGEX = concat!(r"^", IPV4RE!(), r"$");
    pub IP_V6_REGEX = concat!(r"^", IPV6RE!(), r"$");
    pub IP_REGEX = concat!(r"^", IPRE!(), r"$");
    pub CIDR_V4_REGEX =  concat!(r"^", CIDR_V4_REGEX_STR!(), r"$");
    pub CIDR_V6_REGEX =  concat!(r"^", CIDR_V6_REGEX_STR!(), r"$");
    pub CIDR_REGEX =  concat!(r"^(?:", CIDR_V4_REGEX_STR!(), "|",  CIDR_V6_REGEX_STR!(), r")$");
    pub HOSTNAME_REGEX = r"^(?:[a-zA-Z0-9](?:[a-zA-Z0-9\-]*[a-zA-Z0-9])?)$";
    pub DNS_NAME_REGEX =  concat!(r"^", DNS_NAME!(), r"$");
    pub DNS_ALIAS_REGEX =  concat!(r"^", DNS_ALIAS_NAME!(), r"$");
    pub DNS_NAME_OR_IP_REGEX = concat!(r"^(?:", DNS_NAME!(), "|",  IPRE!(), r")$");

    pub SHA256_HEX_REGEX = r"^[a-f0-9]{64}$"; // fixme: define in common_regex ?

    pub PASSWORD_REGEX = r"^[[:^cntrl:]]*$"; // everything but control characters

    pub UUID_REGEX = r"^[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$";

    pub BACKUP_TYPE_REGEX = concat!(r"^(", BACKUP_TYPE_RE!(), r")$");

    pub BACKUP_ID_REGEX = concat!(r"^", BACKUP_ID_RE!(), r"$");

    pub BACKUP_DATE_REGEX = concat!(r"^", BACKUP_TIME_RE!() ,r"$");

    pub GROUP_PATH_REGEX = concat!(r"^(", BACKUP_TYPE_RE!(), ")/(", BACKUP_ID_RE!(), r")$");

    pub BACKUP_FILE_REGEX = r"^.*\.([fd]idx|blob)$";

    pub SNAPSHOT_PATH_REGEX = concat!(r"^", SNAPSHOT_PATH_REGEX_STR!(), r"$");

    pub FINGERPRINT_SHA256_REGEX = r"^(?:[0-9a-fA-F][0-9a-fA-F])(?::[0-9a-fA-F][0-9a-fA-F]){31}$";

    /// Regex for safe identifiers.
    ///
    /// This
    /// [article](https://dwheeler.com/essays/fixing-unix-linux-filenames.html)
    /// contains further information why it is reasonable to restict
    /// names this way. This is not only useful for filenames, but for
    /// any identifier command line tools work with.
    pub PROXMOX_SAFE_ID_REGEX = concat!(r"^", PROXMOX_SAFE_ID_REGEX_STR!(), r"$");

    pub SINGLE_LINE_COMMENT_REGEX = r"^[[:^cntrl:]]*$";

    pub BACKUP_REPO_URL_REGEX = concat!(
        r"^^(?:(?:(",
        USER_ID_REGEX_STR!(), "|", APITOKEN_ID_REGEX_STR!(),
        ")@)?(",
        DNS_NAME!(), "|",  IPRE_BRACKET!(),
        "):)?(?:([0-9]{1,5}):)?(", PROXMOX_SAFE_ID_REGEX_STR!(), r")$"
    );
}

pub const IP_V4_FORMAT: ApiStringFormat = ApiStringFormat::Pattern(&IP_V4_REGEX);
pub const IP_V6_FORMAT: ApiStringFormat = ApiStringFormat::Pattern(&IP_V6_REGEX);
pub const IP_FORMAT: ApiStringFormat = ApiStringFormat::Pattern(&IP_REGEX);
pub const CIDR_V4_FORMAT: ApiStringFormat = ApiStringFormat::Pattern(&CIDR_V4_REGEX);
pub const CIDR_V6_FORMAT: ApiStringFormat = ApiStringFormat::Pattern(&CIDR_V6_REGEX);
pub const CIDR_FORMAT: ApiStringFormat = ApiStringFormat::Pattern(&CIDR_REGEX);

pub const FINGERPRINT_SHA256_FORMAT: ApiStringFormat =
    ApiStringFormat::Pattern(&FINGERPRINT_SHA256_REGEX);

pub const CERT_FINGERPRINT_SHA256_SCHEMA: Schema =
    StringSchema::new("X509 certificate fingerprint (sha256).")
        .format(&FINGERPRINT_SHA256_FORMAT)
        .schema();

pub const PROXMOX_SAFE_ID_FORMAT: ApiStringFormat =
    ApiStringFormat::Pattern(&PROXMOX_SAFE_ID_REGEX);

pub const SINGLE_LINE_COMMENT_FORMAT: ApiStringFormat =
    ApiStringFormat::Pattern(&SINGLE_LINE_COMMENT_REGEX);

pub const SINGLE_LINE_COMMENT_SCHEMA: Schema = StringSchema::new("Comment (single line).")
    .format(&SINGLE_LINE_COMMENT_FORMAT)
    .schema();

pub const BACKUP_ID_FORMAT: ApiStringFormat = ApiStringFormat::Pattern(&BACKUP_ID_REGEX);

/// API schema format definition for repository URLs
pub const BACKUP_REPO_URL: ApiStringFormat = ApiStringFormat::Pattern(&BACKUP_REPO_URL_REGEX);

#[api(
    properties: {
        "upid": {
            optional: true,
            type: UPID,
        },
    },
)]
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
/// Garbage collection status.
pub struct GarbageCollectionStatus {
    pub upid: Option<String>,
    /// Number of processed index files.
    pub index_file_count: usize,
    /// Sum of bytes referred by index files.
    pub index_data_bytes: u64,
    /// Bytes used on disk.
    pub disk_bytes: u64,
    /// Chunks used on disk.
    pub disk_chunks: usize,
    /// Sum of removed bytes.
    pub removed_bytes: u64,
    /// Number of removed chunks.
    pub removed_chunks: usize,
    /// Sum of pending bytes (pending removal - kept for safety).
    pub pending_bytes: u64,
    /// Number of pending chunks (pending removal - kept for safety).
    pub pending_chunks: usize,
    /// Number of chunks marked as .bad by verify that have been removed by GC.
    pub removed_bad: usize,
    /// Number of chunks still marked as .bad after garbage collection.
    pub still_bad: usize,
}

impl Default for GarbageCollectionStatus {
    fn default() -> Self {
        GarbageCollectionStatus {
            upid: None,
            index_file_count: 0,
            index_data_bytes: 0,
            disk_bytes: 0,
            disk_chunks: 0,
            removed_bytes: 0,
            removed_chunks: 0,
            pending_bytes: 0,
            pending_chunks: 0,
            removed_bad: 0,
            still_bad: 0,
        }
    }
}

pub const PVE_CONFIG_DIGEST_FORMAT: ApiStringFormat = ApiStringFormat::Pattern(&SHA256_HEX_REGEX);
pub const CHUNK_DIGEST_FORMAT: ApiStringFormat = ApiStringFormat::Pattern(&SHA256_HEX_REGEX);

pub const PASSWORD_FORMAT: ApiStringFormat = ApiStringFormat::Pattern(&PASSWORD_REGEX);

pub const UUID_FORMAT: ApiStringFormat = ApiStringFormat::Pattern(&UUID_REGEX);
