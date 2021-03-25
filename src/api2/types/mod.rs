//! API Type Definitions

use anyhow::bail;
use serde::{Deserialize, Serialize};

use proxmox::api::{api, schema::*};
use proxmox::const_regex;
use proxmox::{IPRE, IPRE_BRACKET, IPV4RE, IPV6RE, IPV4OCTET, IPV6H16, IPV6LS32};

use crate::{
    backup::{
        CryptMode,
        Fingerprint,
        BACKUP_ID_REGEX,
        DirEntryAttribute,
        CatalogEntryType,
    },
    server::UPID,
    config::acl::Role,
};

#[macro_use]
mod macros;

#[macro_use]
mod userid;
pub use userid::{Realm, RealmRef};
pub use userid::{Tokenname, TokennameRef};
pub use userid::{Username, UsernameRef};
pub use userid::Userid;
pub use userid::Authid;
pub use userid::{PROXMOX_TOKEN_ID_SCHEMA, PROXMOX_TOKEN_NAME_SCHEMA, PROXMOX_GROUP_ID_SCHEMA};

mod tape;
pub use tape::*;

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

macro_rules! DNS_LABEL { () => (r"(?:[a-zA-Z0-9](?:[a-zA-Z0-9\-]*[a-zA-Z0-9])?)") }
macro_rules! DNS_NAME { () => (concat!(r"(?:(?:", DNS_LABEL!() , r"\.)*", DNS_LABEL!(), ")")) }

macro_rules! CIDR_V4_REGEX_STR { () => (concat!(r"(?:", IPV4RE!(), r"/\d{1,2})$")) }
macro_rules! CIDR_V6_REGEX_STR { () => (concat!(r"(?:", IPV6RE!(), r"/\d{1,3})$")) }

const_regex!{
    pub IP_V4_REGEX = concat!(r"^", IPV4RE!(), r"$");
    pub IP_V6_REGEX = concat!(r"^", IPV6RE!(), r"$");
    pub IP_REGEX = concat!(r"^", IPRE!(), r"$");
    pub CIDR_V4_REGEX =  concat!(r"^", CIDR_V4_REGEX_STR!(), r"$");
    pub CIDR_V6_REGEX =  concat!(r"^", CIDR_V6_REGEX_STR!(), r"$");
    pub CIDR_REGEX =  concat!(r"^(?:", CIDR_V4_REGEX_STR!(), "|",  CIDR_V6_REGEX_STR!(), r")$");

    pub SHA256_HEX_REGEX = r"^[a-f0-9]{64}$"; // fixme: define in common_regex ?
    pub SYSTEMD_DATETIME_REGEX = r"^\d{4}-\d{2}-\d{2}( \d{2}:\d{2}(:\d{2})?)?$"; //  fixme: define in common_regex ?

    pub PASSWORD_REGEX = r"^[[:^cntrl:]]*$"; // everything but control characters

    /// Regex for safe identifiers.
    ///
    /// This
    /// [article](https://dwheeler.com/essays/fixing-unix-linux-filenames.html)
    /// contains further information why it is reasonable to restict
    /// names this way. This is not only useful for filenames, but for
    /// any identifier command line tools work with.
    pub PROXMOX_SAFE_ID_REGEX = concat!(r"^", PROXMOX_SAFE_ID_REGEX_STR!(), r"$");

    /// Regex for verification jobs 'DATASTORE:ACTUAL_JOB_ID'
    pub VERIFICATION_JOB_WORKER_ID_REGEX = concat!(r"^(", PROXMOX_SAFE_ID_REGEX_STR!(), r"):");
    /// Regex for sync jobs 'REMOTE:REMOTE_DATASTORE:LOCAL_DATASTORE:ACTUAL_JOB_ID'
    pub SYNC_JOB_WORKER_ID_REGEX = concat!(r"^(", PROXMOX_SAFE_ID_REGEX_STR!(), r"):(", PROXMOX_SAFE_ID_REGEX_STR!(), r"):(", PROXMOX_SAFE_ID_REGEX_STR!(), r"):");

    pub SINGLE_LINE_COMMENT_REGEX = r"^[[:^cntrl:]]*$";

    pub HOSTNAME_REGEX = r"^(?:[a-zA-Z0-9](?:[a-zA-Z0-9\-]*[a-zA-Z0-9])?)$";

    pub DNS_NAME_REGEX =  concat!(r"^", DNS_NAME!(), r"$");

    pub DNS_NAME_OR_IP_REGEX = concat!(r"^(?:", DNS_NAME!(), "|",  IPRE!(), r")$");

    pub BACKUP_REPO_URL_REGEX = concat!(r"^^(?:(?:(", USER_ID_REGEX_STR!(), "|", APITOKEN_ID_REGEX_STR!(), ")@)?(", DNS_NAME!(), "|",  IPRE_BRACKET!() ,"):)?(?:([0-9]{1,5}):)?(", PROXMOX_SAFE_ID_REGEX_STR!(), r")$");

    pub FINGERPRINT_SHA256_REGEX = r"^(?:[0-9a-fA-F][0-9a-fA-F])(?::[0-9a-fA-F][0-9a-fA-F]){31}$";

    pub ACL_PATH_REGEX = concat!(r"^(?:/|", r"(?:/", PROXMOX_SAFE_ID_REGEX_STR!(), ")+", r")$");

    pub SUBSCRIPTION_KEY_REGEX = concat!(r"^pbs(?:[cbsp])-[0-9a-f]{10}$");

    pub BLOCKDEVICE_NAME_REGEX = r"^(:?(:?h|s|x?v)d[a-z]+)|(:?nvme\d+n\d+)$";

    pub ZPOOL_NAME_REGEX = r"^[a-zA-Z][a-z0-9A-Z\-_.:]+$";

    pub UUID_REGEX = r"^[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$";

    pub DATASTORE_MAP_REGEX = concat!(r"(:?", PROXMOX_SAFE_ID_REGEX_STR!(), r"=)?", PROXMOX_SAFE_ID_REGEX_STR!());
}

pub const SYSTEMD_DATETIME_FORMAT: ApiStringFormat =
    ApiStringFormat::Pattern(&SYSTEMD_DATETIME_REGEX);

pub const IP_V4_FORMAT: ApiStringFormat =
    ApiStringFormat::Pattern(&IP_V4_REGEX);

pub const IP_V6_FORMAT: ApiStringFormat =
    ApiStringFormat::Pattern(&IP_V6_REGEX);

pub const IP_FORMAT: ApiStringFormat =
    ApiStringFormat::Pattern(&IP_REGEX);

pub const PVE_CONFIG_DIGEST_FORMAT: ApiStringFormat =
    ApiStringFormat::Pattern(&SHA256_HEX_REGEX);

pub const FINGERPRINT_SHA256_FORMAT: ApiStringFormat =
    ApiStringFormat::Pattern(&FINGERPRINT_SHA256_REGEX);

pub const PROXMOX_SAFE_ID_FORMAT: ApiStringFormat =
    ApiStringFormat::Pattern(&PROXMOX_SAFE_ID_REGEX);

pub const BACKUP_ID_FORMAT: ApiStringFormat =
    ApiStringFormat::Pattern(&BACKUP_ID_REGEX);

pub const UUID_FORMAT: ApiStringFormat =
    ApiStringFormat::Pattern(&UUID_REGEX);

pub const SINGLE_LINE_COMMENT_FORMAT: ApiStringFormat =
    ApiStringFormat::Pattern(&SINGLE_LINE_COMMENT_REGEX);

pub const HOSTNAME_FORMAT: ApiStringFormat =
    ApiStringFormat::Pattern(&HOSTNAME_REGEX);

pub const DNS_NAME_FORMAT: ApiStringFormat =
    ApiStringFormat::Pattern(&DNS_NAME_REGEX);

pub const DNS_NAME_OR_IP_FORMAT: ApiStringFormat =
    ApiStringFormat::Pattern(&DNS_NAME_OR_IP_REGEX);

pub const PASSWORD_FORMAT: ApiStringFormat =
    ApiStringFormat::Pattern(&PASSWORD_REGEX);

pub const ACL_PATH_FORMAT: ApiStringFormat =
    ApiStringFormat::Pattern(&ACL_PATH_REGEX);

pub const NETWORK_INTERFACE_FORMAT: ApiStringFormat =
    ApiStringFormat::Pattern(&PROXMOX_SAFE_ID_REGEX);

pub const CIDR_V4_FORMAT: ApiStringFormat =
    ApiStringFormat::Pattern(&CIDR_V4_REGEX);

pub const CIDR_V6_FORMAT: ApiStringFormat =
    ApiStringFormat::Pattern(&CIDR_V6_REGEX);

pub const CIDR_FORMAT: ApiStringFormat =
    ApiStringFormat::Pattern(&CIDR_REGEX);

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

pub const CERT_FINGERPRINT_SHA256_SCHEMA: Schema = StringSchema::new(
    "X509 certificate fingerprint (sha256)."
)
    .format(&FINGERPRINT_SHA256_FORMAT)
    .schema();

pub const TAPE_ENCRYPTION_KEY_FINGERPRINT_SCHEMA: Schema = StringSchema::new(
    "Tape encryption key fingerprint (sha256)."
)
    .format(&FINGERPRINT_SHA256_FORMAT)
    .schema();

pub const PROXMOX_CONFIG_DIGEST_SCHEMA: Schema = StringSchema::new(
    "Prevent changes if current configuration file has different \
    SHA256 digest. This can be used to prevent concurrent \
    modifications."
)
    .format(&PVE_CONFIG_DIGEST_FORMAT) .schema();


pub const CHUNK_DIGEST_FORMAT: ApiStringFormat =
    ApiStringFormat::Pattern(&SHA256_HEX_REGEX);

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

pub const IP_V4_SCHEMA: Schema =
    StringSchema::new("IPv4 address.")
    .format(&IP_V4_FORMAT)
    .max_length(15)
    .schema();

pub const IP_V6_SCHEMA: Schema =
    StringSchema::new("IPv6 address.")
    .format(&IP_V6_FORMAT)
    .max_length(39)
    .schema();

pub const IP_SCHEMA: Schema =
    StringSchema::new("IP (IPv4 or IPv6) address.")
    .format(&IP_FORMAT)
    .max_length(39)
    .schema();

pub const CIDR_V4_SCHEMA: Schema =
    StringSchema::new("IPv4 address with netmask (CIDR notation).")
    .format(&CIDR_V4_FORMAT)
    .max_length(18)
    .schema();

pub const CIDR_V6_SCHEMA: Schema =
    StringSchema::new("IPv6 address with netmask (CIDR notation).")
    .format(&CIDR_V6_FORMAT)
    .max_length(43)
    .schema();

pub const CIDR_SCHEMA: Schema =
    StringSchema::new("IP address (IPv4 or IPv6) with netmask (CIDR notation).")
    .format(&CIDR_FORMAT)
    .max_length(43)
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

pub const BACKUP_ARCHIVE_NAME_SCHEMA: Schema =
    StringSchema::new("Backup archive name.")
    .format(&PROXMOX_SAFE_ID_FORMAT)
    .schema();

pub const BACKUP_TYPE_SCHEMA: Schema =
    StringSchema::new("Backup type.")
    .format(&ApiStringFormat::Enum(&[
        EnumEntry::new("vm", "Virtual Machine Backup"),
        EnumEntry::new("ct", "Container Backup"),
        EnumEntry::new("host", "Host Backup")]))
    .schema();

pub const BACKUP_ID_SCHEMA: Schema =
    StringSchema::new("Backup ID.")
    .format(&BACKUP_ID_FORMAT)
    .schema();

pub const BACKUP_TIME_SCHEMA: Schema =
    IntegerSchema::new("Backup time (Unix epoch.)")
    .minimum(1_547_797_308)
    .schema();

pub const UPID_SCHEMA: Schema = StringSchema::new("Unique Process/Task ID.")
    .max_length(256)
    .schema();

pub const DATASTORE_SCHEMA: Schema = StringSchema::new("Datastore name.")
    .format(&PROXMOX_SAFE_ID_FORMAT)
    .min_length(3)
    .max_length(32)
    .schema();

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

pub const MEDIA_SET_UUID_SCHEMA: Schema =
    StringSchema::new("MediaSet Uuid (We use the all-zero Uuid to reseve an empty media for a specific pool).")
    .format(&UUID_FORMAT)
    .schema();

pub const MEDIA_UUID_SCHEMA: Schema =
    StringSchema::new("Media Uuid.")
    .format(&UUID_FORMAT)
    .schema();

pub const SYNC_SCHEDULE_SCHEMA: Schema = StringSchema::new(
    "Run sync job at specified schedule.")
    .format(&ApiStringFormat::VerifyFn(crate::tools::systemd::time::verify_calendar_event))
    .type_text("<calendar-event>")
    .schema();

pub const GC_SCHEDULE_SCHEMA: Schema = StringSchema::new(
    "Run garbage collection job at specified schedule.")
    .format(&ApiStringFormat::VerifyFn(crate::tools::systemd::time::verify_calendar_event))
    .type_text("<calendar-event>")
    .schema();

pub const PRUNE_SCHEDULE_SCHEMA: Schema = StringSchema::new(
    "Run prune job at specified schedule.")
    .format(&ApiStringFormat::VerifyFn(crate::tools::systemd::time::verify_calendar_event))
    .type_text("<calendar-event>")
    .schema();

pub const VERIFICATION_SCHEDULE_SCHEMA: Schema = StringSchema::new(
    "Run verify job at specified schedule.")
    .format(&ApiStringFormat::VerifyFn(crate::tools::systemd::time::verify_calendar_event))
    .type_text("<calendar-event>")
    .schema();

pub const REMOTE_ID_SCHEMA: Schema = StringSchema::new("Remote ID.")
    .format(&PROXMOX_SAFE_ID_FORMAT)
    .min_length(3)
    .max_length(32)
    .schema();

pub const JOB_ID_SCHEMA: Schema = StringSchema::new("Job ID.")
    .format(&PROXMOX_SAFE_ID_FORMAT)
    .min_length(3)
    .max_length(32)
    .schema();

pub const REMOVE_VANISHED_BACKUPS_SCHEMA: Schema = BooleanSchema::new(
    "Delete vanished backups. This remove the local copy if the remote backup was deleted.")
    .default(true)
    .schema();

pub const IGNORE_VERIFIED_BACKUPS_SCHEMA: Schema = BooleanSchema::new(
    "Do not verify backups that are already verified if their verification is not outdated.")
    .default(true)
    .schema();

pub const VERIFICATION_OUTDATED_AFTER_SCHEMA: Schema = IntegerSchema::new(
    "Days after that a verification becomes outdated")
    .minimum(1)
    .schema();

pub const SINGLE_LINE_COMMENT_SCHEMA: Schema = StringSchema::new("Comment (single line).")
    .format(&SINGLE_LINE_COMMENT_FORMAT)
    .schema();

pub const HOSTNAME_SCHEMA: Schema = StringSchema::new("Hostname (as defined in RFC1123).")
    .format(&HOSTNAME_FORMAT)
    .schema();

pub const DNS_NAME_OR_IP_SCHEMA: Schema = StringSchema::new("DNS name or IP address.")
    .format(&DNS_NAME_OR_IP_FORMAT)
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
        store: {
            schema: DATASTORE_SCHEMA,
        },
        comment: {
            optional: true,
            schema: SINGLE_LINE_COMMENT_SCHEMA,
        },
    },
)]
#[derive(Serialize, Deserialize)]
#[serde(rename_all="kebab-case")]
/// Basic information about a datastore.
pub struct DataStoreListItem {
    pub store: String,
    pub comment: Option<String>,
}

#[api(
    properties: {
        "backup-type": {
            schema: BACKUP_TYPE_SCHEMA,
        },
        "backup-id": {
            schema: BACKUP_ID_SCHEMA,
        },
        "last-backup": {
            schema: BACKUP_TIME_SCHEMA,
        },
        "backup-count": {
            type: Integer,
        },
        files: {
            items: {
                schema: BACKUP_ARCHIVE_NAME_SCHEMA
            },
        },
        owner: {
            type: Authid,
            optional: true,
        },
    },
)]
#[derive(Serialize, Deserialize)]
#[serde(rename_all="kebab-case")]
/// Basic information about a backup group.
pub struct GroupListItem {
    pub backup_type: String, // enum
    pub backup_id: String,
    pub last_backup: i64,
    /// Number of contained snapshots
    pub backup_count: u64,
    /// List of contained archive files.
    pub files: Vec<String>,
    /// The owner of group
    #[serde(skip_serializing_if="Option::is_none")]
    pub owner: Option<Authid>,
}

#[api()]
#[derive(Debug, Copy, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
/// Result of a verify operation.
pub enum VerifyState {
    /// Verification was successful
    Ok,
    /// Verification reported one or more errors
    Failed,
}

#[api(
    properties: {
        upid: {
            schema: UPID_SCHEMA
        },
        state: {
            type: VerifyState
        },
    },
)]
#[derive(Serialize, Deserialize)]
/// Task properties.
pub struct SnapshotVerifyState {
    /// UPID of the verify task
    pub upid: UPID,
    /// State of the verification. Enum.
    pub state: VerifyState,
}

#[api(
    properties: {
        "backup-type": {
            schema: BACKUP_TYPE_SCHEMA,
        },
        "backup-id": {
            schema: BACKUP_ID_SCHEMA,
        },
        "backup-time": {
            schema: BACKUP_TIME_SCHEMA,
        },
        comment: {
            schema: SINGLE_LINE_COMMENT_SCHEMA,
            optional: true,
        },
        verification: {
            type: SnapshotVerifyState,
            optional: true,
        },
        fingerprint: {
            type: String,
            optional: true,
        },
        files: {
            items: {
                schema: BACKUP_ARCHIVE_NAME_SCHEMA
            },
        },
        owner: {
            type: Authid,
            optional: true,
        },
    },
)]
#[derive(Serialize, Deserialize)]
#[serde(rename_all="kebab-case")]
/// Basic information about backup snapshot.
pub struct SnapshotListItem {
    pub backup_type: String, // enum
    pub backup_id: String,
    pub backup_time: i64,
    /// The first line from manifest "notes"
    #[serde(skip_serializing_if="Option::is_none")]
    pub comment: Option<String>,
    /// The result of the last run verify task
    #[serde(skip_serializing_if="Option::is_none")]
    pub verification: Option<SnapshotVerifyState>,
    /// Fingerprint of encryption key
    #[serde(skip_serializing_if="Option::is_none")]
    pub fingerprint: Option<Fingerprint>,
    /// List of contained archive files.
    pub files: Vec<BackupContent>,
    /// Overall snapshot size (sum of all archive sizes).
    #[serde(skip_serializing_if="Option::is_none")]
    pub size: Option<u64>,
    /// The owner of the snapshots group
    #[serde(skip_serializing_if="Option::is_none")]
    pub owner: Option<Authid>,
}

#[api(
    properties: {
        "backup-type": {
            schema: BACKUP_TYPE_SCHEMA,
        },
        "backup-id": {
            schema: BACKUP_ID_SCHEMA,
        },
        "backup-time": {
            schema: BACKUP_TIME_SCHEMA,
        },
    },
)]
#[derive(Serialize, Deserialize)]
#[serde(rename_all="kebab-case")]
/// Prune result.
pub struct PruneListItem {
    pub backup_type: String, // enum
    pub backup_id: String,
    pub backup_time: i64,
    /// Keep snapshot
    pub keep: bool,
}

pub const PRUNE_SCHEMA_KEEP_DAILY: Schema = IntegerSchema::new(
    "Number of daily backups to keep.")
    .minimum(1)
    .schema();

pub const PRUNE_SCHEMA_KEEP_HOURLY: Schema = IntegerSchema::new(
    "Number of hourly backups to keep.")
    .minimum(1)
    .schema();

pub const PRUNE_SCHEMA_KEEP_LAST: Schema = IntegerSchema::new(
    "Number of backups to keep.")
    .minimum(1)
    .schema();

pub const PRUNE_SCHEMA_KEEP_MONTHLY: Schema = IntegerSchema::new(
    "Number of monthly backups to keep.")
    .minimum(1)
    .schema();

pub const PRUNE_SCHEMA_KEEP_WEEKLY: Schema = IntegerSchema::new(
    "Number of weekly backups to keep.")
    .minimum(1)
    .schema();

pub const PRUNE_SCHEMA_KEEP_YEARLY: Schema = IntegerSchema::new(
    "Number of yearly backups to keep.")
    .minimum(1)
    .schema();

#[api(
    properties: {
        "filename": {
            schema: BACKUP_ARCHIVE_NAME_SCHEMA,
        },
        "crypt-mode": {
            type: CryptMode,
            optional: true,
        },
    },
)]
#[derive(Serialize, Deserialize)]
#[serde(rename_all="kebab-case")]
/// Basic information about archive files inside a backup snapshot.
pub struct BackupContent {
    pub filename: String,
    /// Info if file is encrypted, signed, or neither.
    #[serde(skip_serializing_if="Option::is_none")]
    pub crypt_mode: Option<CryptMode>,
    /// Archive size (from backup manifest).
    #[serde(skip_serializing_if="Option::is_none")]
    pub size: Option<u64>,
}

#[api(
    properties: {
        "upid": {
            optional: true,
            schema: UPID_SCHEMA,
        },
    },
)]
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all="kebab-case")]
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


#[api()]
#[derive(Serialize, Deserialize)]
/// Storage space usage information.
pub struct StorageStatus {
    /// Total space (bytes).
    pub total: u64,
    /// Used space (bytes).
    pub used: u64,
    /// Available space (bytes).
    pub avail: u64,
}

#[api()]
#[derive(Serialize, Deserialize, Default)]
/// Backup Type group/snapshot counts.
pub struct TypeCounts {
    /// The number of groups of the type.
    pub groups: u64,
    /// The number of snapshots of the type.
    pub snapshots: u64,
}

#[api(
    properties: {
        ct: {
            type: TypeCounts,
            optional: true,
        },
        host: {
            type: TypeCounts,
            optional: true,
        },
        vm: {
            type: TypeCounts,
            optional: true,
        },
        other: {
            type: TypeCounts,
            optional: true,
        },
    },
)]
#[derive(Serialize, Deserialize, Default)]
/// Counts of groups/snapshots per BackupType.
pub struct Counts {
    /// The counts for CT backups
    pub ct: Option<TypeCounts>,
    /// The counts for Host backups
    pub host: Option<TypeCounts>,
    /// The counts for VM backups
    pub vm: Option<TypeCounts>,
    /// The counts for other backup types
    pub other: Option<TypeCounts>,
}

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

#[api(
    properties: {
        upid: { schema: UPID_SCHEMA },
        user: { type: Authid },
    },
)]
#[derive(Serialize, Deserialize)]
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
    pub user: Authid,
    /// The task end time (Epoch)
    #[serde(skip_serializing_if="Option::is_none")]
    pub endtime: Option<i64>,
    /// Task end status
    #[serde(skip_serializing_if="Option::is_none")]
    pub status: Option<String>,
}

impl From<crate::server::TaskListInfo> for TaskListItem {
    fn from(info: crate::server::TaskListInfo) -> Self {
        let (endtime, status) = info
            .state
            .map_or_else(|| (None, None), |a| (Some(a.endtime()), Some(a.to_string())));

        TaskListItem {
            upid: info.upid_str,
            node: "localhost".to_string(),
            pid: info.upid.pid as i64,
            pstart: info.upid.pstart,
            starttime: info.upid.starttime,
            worker_type: info.upid.worker_type,
            worker_id: info.upid.worker_id,
            user: info.upid.auth_id,
            endtime,
            status,
        }
    }
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

#[api()]
#[derive(Debug, Copy, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
/// Interface configuration method
pub enum NetworkConfigMethod {
    /// Configuration is done manually using other tools
    Manual,
    /// Define interfaces with statically allocated addresses.
    Static,
    /// Obtain an address via DHCP
    DHCP,
    /// Define the loopback interface.
    Loopback,
}

#[api()]
#[derive(Debug, Copy, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[allow(non_camel_case_types)]
#[repr(u8)]
/// Linux Bond Mode
pub enum LinuxBondMode {
    /// Round-robin policy
    balance_rr = 0,
    /// Active-backup policy
    active_backup = 1,
    /// XOR policy
    balance_xor = 2,
    /// Broadcast policy
    broadcast = 3,
    /// IEEE 802.3ad Dynamic link aggregation
    #[serde(rename = "802.3ad")]
    ieee802_3ad = 4,
    /// Adaptive transmit load balancing
    balance_tlb = 5,
    /// Adaptive load balancing
    balance_alb = 6,
}

#[api()]
#[derive(Debug, Copy, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[allow(non_camel_case_types)]
#[repr(u8)]
/// Bond Transmit Hash Policy for LACP (802.3ad)
pub enum BondXmitHashPolicy {
    /// Layer 2
    layer2 = 0,
    /// Layer 2+3
    #[serde(rename = "layer2+3")]
    layer2_3 = 1,
    /// Layer 3+4
    #[serde(rename = "layer3+4")]
    layer3_4 = 2,
}

#[api()]
#[derive(Debug, Copy, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
/// Network interface type
pub enum NetworkInterfaceType {
    /// Loopback
    Loopback,
    /// Physical Ethernet device
    Eth,
    /// Linux Bridge
    Bridge,
    /// Linux Bond
    Bond,
    /// Linux VLAN (eth.10)
    Vlan,
    /// Interface Alias (eth:1)
    Alias,
    /// Unknown interface type
    Unknown,
}

pub const NETWORK_INTERFACE_NAME_SCHEMA: Schema = StringSchema::new("Network interface name.")
    .format(&NETWORK_INTERFACE_FORMAT)
    .min_length(1)
    .max_length(libc::IFNAMSIZ-1)
    .schema();

pub const NETWORK_INTERFACE_ARRAY_SCHEMA: Schema = ArraySchema::new(
    "Network interface list.", &NETWORK_INTERFACE_NAME_SCHEMA)
    .schema();

pub const NETWORK_INTERFACE_LIST_SCHEMA: Schema = StringSchema::new(
    "A list of network devices, comma separated.")
    .format(&ApiStringFormat::PropertyString(&NETWORK_INTERFACE_ARRAY_SCHEMA))
    .schema();

#[api(
    properties: {
        name: {
            schema: NETWORK_INTERFACE_NAME_SCHEMA,
        },
        "type": {
            type: NetworkInterfaceType,
        },
        method: {
            type: NetworkConfigMethod,
            optional: true,
        },
        method6: {
            type: NetworkConfigMethod,
            optional: true,
        },
        cidr: {
            schema: CIDR_V4_SCHEMA,
            optional: true,
        },
        cidr6: {
            schema: CIDR_V6_SCHEMA,
            optional: true,
        },
        gateway: {
            schema: IP_V4_SCHEMA,
            optional: true,
        },
        gateway6: {
            schema: IP_V6_SCHEMA,
            optional: true,
        },
        options: {
            description: "Option list (inet)",
            type: Array,
            items: {
                description: "Optional attribute line.",
                type: String,
            },
        },
        options6: {
            description: "Option list (inet6)",
            type: Array,
            items: {
                description: "Optional attribute line.",
                type: String,
            },
        },
        comments: {
            description: "Comments (inet, may span multiple lines)",
            type: String,
            optional: true,
        },
        comments6: {
            description: "Comments (inet6, may span multiple lines)",
            type: String,
            optional: true,
        },
        bridge_ports: {
            schema: NETWORK_INTERFACE_ARRAY_SCHEMA,
            optional: true,
        },
        slaves: {
            schema: NETWORK_INTERFACE_ARRAY_SCHEMA,
            optional: true,
        },
        bond_mode: {
            type: LinuxBondMode,
            optional: true,
        },
        "bond-primary": {
            schema: NETWORK_INTERFACE_NAME_SCHEMA,
            optional: true,
        },
        bond_xmit_hash_policy: {
            type: BondXmitHashPolicy,
            optional: true,
        },
    }
)]
#[derive(Debug, Serialize, Deserialize)]
/// Network Interface configuration
pub struct Interface {
    /// Autostart interface
    #[serde(rename = "autostart")]
    pub autostart: bool,
    /// Interface is active (UP)
    pub active: bool,
    /// Interface name
    pub name: String,
    /// Interface type
    #[serde(rename = "type")]
    pub interface_type: NetworkInterfaceType,
    #[serde(skip_serializing_if="Option::is_none")]
    pub method: Option<NetworkConfigMethod>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub method6: Option<NetworkConfigMethod>,
    #[serde(skip_serializing_if="Option::is_none")]
    /// IPv4 address with netmask
    pub cidr: Option<String>,
    #[serde(skip_serializing_if="Option::is_none")]
    /// IPv4 gateway
    pub gateway: Option<String>,
    #[serde(skip_serializing_if="Option::is_none")]
    /// IPv6 address with netmask
    pub cidr6: Option<String>,
    #[serde(skip_serializing_if="Option::is_none")]
    /// IPv6 gateway
    pub gateway6: Option<String>,

    #[serde(skip_serializing_if="Vec::is_empty")]
    pub options: Vec<String>,
    #[serde(skip_serializing_if="Vec::is_empty")]
    pub options6: Vec<String>,

    #[serde(skip_serializing_if="Option::is_none")]
    pub comments: Option<String>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub comments6: Option<String>,

    #[serde(skip_serializing_if="Option::is_none")]
    /// Maximum Transmission Unit
    pub mtu: Option<u64>,

    #[serde(skip_serializing_if="Option::is_none")]
    pub bridge_ports: Option<Vec<String>>,
    /// Enable bridge vlan support.
    #[serde(skip_serializing_if="Option::is_none")]
    pub bridge_vlan_aware: Option<bool>,

    #[serde(skip_serializing_if="Option::is_none")]
    pub slaves: Option<Vec<String>>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub bond_mode: Option<LinuxBondMode>,
    #[serde(skip_serializing_if="Option::is_none")]
    #[serde(rename = "bond-primary")]
    pub bond_primary: Option<String>,
    pub bond_xmit_hash_policy: Option<BondXmitHashPolicy>,
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
    /// URL under which the package's changelog can be retrieved
    pub change_log_url: String,
    /// Custom extra field for additional package information
    #[serde(skip_serializing_if="Option::is_none")]
    pub extra_info: Option<String>,
}

#[api()]
#[derive(Debug, Copy, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
/// When do we send notifications
pub enum Notify {
    /// Never send notification
    Never,
    /// Send notifications for failed and successful jobs
    Always,
    /// Send notifications for failed jobs only
    Error,
}

#[api(
    properties: {
        gc: {
            type: Notify,
            optional: true,
        },
        verify: {
            type: Notify,
            optional: true,
        },
        sync: {
            type: Notify,
            optional: true,
        },
    },
)]
#[derive(Debug, Serialize, Deserialize)]
/// Datastore notify settings
pub struct DatastoreNotify {
    /// Garbage collection settings
    pub gc: Option<Notify>,
    /// Verify job setting
    pub verify: Option<Notify>,
    /// Sync job setting
    pub sync: Option<Notify>,
}

/// An entry in a hierarchy of files for restore and listing.
#[api()]
#[derive(Serialize, Deserialize)]
pub struct ArchiveEntry {
    /// Base64-encoded full path to the file, including the filename
    pub filepath: String,
    /// Displayable filename text for UIs
    pub text: String,
    /// File or directory type of this entry
    #[serde(rename = "type")]
    pub entry_type: String,
    /// Is this entry a leaf node, or does it have children (i.e. a directory)?
    pub leaf: bool,
    /// The file size, if entry_type is 'f' (file)
    #[serde(skip_serializing_if="Option::is_none")]
    pub size: Option<u64>,
    /// The file "last modified" time stamp, if entry_type is 'f' (file)
    #[serde(skip_serializing_if="Option::is_none")]
    pub mtime: Option<i64>,
}

impl ArchiveEntry {
    pub fn new(filepath: &[u8], entry_type: &DirEntryAttribute) -> Self {
        Self {
            filepath: base64::encode(filepath),
            text: String::from_utf8_lossy(filepath.split(|x| *x == b'/').last().unwrap())
                .to_string(),
            entry_type: CatalogEntryType::from(entry_type).to_string(),
            leaf: !matches!(entry_type, DirEntryAttribute::Directory { .. }),
            size: match entry_type {
                DirEntryAttribute::File { size, .. } => Some(*size),
                _ => None
            },
            mtime: match entry_type {
                DirEntryAttribute::File { mtime, .. } => Some(*mtime),
                _ => None
            },
        }
    }
}

pub const DATASTORE_NOTIFY_STRING_SCHEMA: Schema = StringSchema::new(
    "Datastore notification setting")
    .format(&ApiStringFormat::PropertyString(&DatastoreNotify::API_SCHEMA))
    .schema();


pub const PASSWORD_HINT_SCHEMA: Schema = StringSchema::new("Password hint.")
    .format(&SINGLE_LINE_COMMENT_FORMAT)
    .min_length(1)
    .max_length(64)
    .schema();

#[api(default: "scrypt")]
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
/// Key derivation function for password protected encryption keys.
pub enum Kdf {
    /// Do not encrypt the key.
    None,
    /// Encrypt they key with a password using SCrypt.
    Scrypt,
    /// Encrtypt the Key with a password using PBKDF2
    PBKDF2,
}

impl Default for Kdf {
    #[inline]
    fn default() -> Self {
        Kdf::Scrypt
    }
}

#[api(
    properties: {
        kdf: {
            type: Kdf,
        },
        fingerprint: {
            schema: CERT_FINGERPRINT_SHA256_SCHEMA,
            optional: true,
        },
    },
)]
#[derive(Deserialize, Serialize)]
/// Encryption Key Information
pub struct KeyInfo {
    /// Path to key (if stored in a file)
    #[serde(skip_serializing_if="Option::is_none")]
    pub path: Option<String>,
    pub kdf: Kdf,
    /// Key creation time
    pub created: i64,
    /// Key modification time
    pub modified: i64,
    /// Key fingerprint
    #[serde(skip_serializing_if="Option::is_none")]
    pub fingerprint: Option<String>,
    /// Password hint
    #[serde(skip_serializing_if="Option::is_none")]
    pub hint: Option<String>,
}

#[api]
#[derive(Deserialize, Serialize)]
/// RSA public key information
pub struct RsaPubKeyInfo {
    /// Path to key (if stored in a file)
    #[serde(skip_serializing_if="Option::is_none")]
    pub path: Option<String>,
    /// RSA exponent
    pub exponent: String,
    /// Hex-encoded RSA modulus
    pub modulus: String,
    /// Key (modulus) length in bits
    pub length: usize,
}

impl std::convert::TryFrom<openssl::rsa::Rsa<openssl::pkey::Public>> for RsaPubKeyInfo {
    type Error = anyhow::Error;

    fn try_from(value: openssl::rsa::Rsa<openssl::pkey::Public>) -> Result<Self, Self::Error> {
        let modulus = value.n().to_hex_str()?.to_string();
        let exponent = value.e().to_dec_str()?.to_string();
        let length = value.size() as usize * 8;

        Ok(Self {
            path: None,
            exponent,
            modulus,
            length,
        })
    }
}

#[api(
    properties: {
        "next-run": {
            description: "Estimated time of the next run (UNIX epoch).",
            optional: true,
            type: Integer,
        },
        "last-run-state": {
            description: "Result of the last run.",
            optional: true,
            type: String,
        },
        "last-run-upid": {
            description: "Task UPID of the last run.",
            optional: true,
            type: String,
        },
        "last-run-endtime": {
            description: "Endtime of the last run.",
            optional: true,
            type: Integer,
        },
    }
)]
#[serde(rename_all="kebab-case")]
#[derive(Serialize,Deserialize,Default)]
/// Job Scheduling Status
pub struct JobScheduleStatus {
    #[serde(skip_serializing_if="Option::is_none")]
    pub next_run: Option<i64>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub last_run_state: Option<String>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub last_run_upid: Option<String>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub last_run_endtime: Option<i64>,
}
