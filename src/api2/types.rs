use failure::*;
use ::serde::{Deserialize, Serialize};

use proxmox::api::{api, const_regex, schema::*};
use proxmox::tools::*; // required to use IPRE!() macro ???

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
    pub IP_FORMAT_REGEX = IPRE!();
    pub SHA256_HEX_REGEX = r"^[a-f0-9]{64}$"; // fixme: define in common_regex ?
    pub SYSTEMD_DATETIME_REGEX = r"^\d{4}-\d{2}-\d{2}( \d{2}:\d{2}(:\d{2})?)?$"; //  fixme: define in common_regex ?

    /// Regex for safe identifiers.
    ///
    /// This
    /// [article](https://dwheeler.com/essays/fixing-unix-linux-filenames.html)
    /// contains further information why it is reasonable to restict
    /// names this way. This is not only useful for filenames, but for
    /// any identifier command line tools work with.
    pub PROXMOX_SAFE_ID_REGEX = r"^[A-Za-z0-9_][A-Za-z0-9._\-]*";
}

pub const SYSTEMD_DATETIME_FORMAT: ApiStringFormat =
    ApiStringFormat::Pattern(&SYSTEMD_DATETIME_REGEX);

pub const IP_FORMAT: ApiStringFormat =
    ApiStringFormat::Pattern(&IP_FORMAT_REGEX);

pub const PVE_CONFIG_DIGEST_FORMAT: ApiStringFormat =
    ApiStringFormat::Pattern(&SHA256_HEX_REGEX);

pub const PROXMOX_SAFE_ID_FORMAT: ApiStringFormat =
    ApiStringFormat::Pattern(&PROXMOX_SAFE_ID_REGEX);

pub const PVE_CONFIG_DIGEST_SCHEMA: Schema = StringSchema::new(r#"\
Prevent changes if current configuration file has different SHA256 digest.
This can be used to prevent concurrent modifications.
"#
)
    .format(&PVE_CONFIG_DIGEST_FORMAT)
    .schema();


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

pub const BACKUP_ARCHIVE_NAME_SCHEMA: Schema =
    StringSchema::new("Backup archive name.")
    .format(&PROXMOX_SAFE_ID_FORMAT)
    .schema();

pub const BACKUP_TYPE_SCHEMA: Schema =
    StringSchema::new("Backup type.")
    .format(&ApiStringFormat::Enum(&["vm", "ct", "host"]))
    .schema();

pub const BACKUP_ID_SCHEMA: Schema =
    StringSchema::new("Backup ID.")
    .format(&PROXMOX_SAFE_ID_FORMAT)
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
    .max_length(32)
    .schema();



// Complex type definitions

#[api(
    description: "Basic information about backup snapshot.",
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
pub struct SnapshotListItem {
    pub backup_type: String, // enum
    pub backup_id: String,
    pub backup_time: i64,
    pub files: Vec<String>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub size: Option<u64>,
}
