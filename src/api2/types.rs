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

macro_rules! DNS_LABEL { () => (r"(?:[a-zA-Z0-9](?:[a-zA-Z0-9\-]*[a-zA-Z0-9])?)") }
macro_rules! DNS_NAME { () => (concat!(r"(?:", DNS_LABEL!() , r"\.)*", DNS_LABEL!())) }

// we only allow a limited set of characters
// colon is not allowed, because we store usernames in
// colon separated lists)!
// slash is not allowed because it is used as pve API delimiter
// also see "man useradd"
macro_rules! USER_NAME_REGEX_STR { () => (r"(?:[^\s:/[:cntrl:]]+)") }

macro_rules! PROXMOX_SAFE_ID_REGEX_STR {  () => (r"(?:[A-Za-z0-9_][A-Za-z0-9._\-]*)") }

const_regex!{
    pub IP_FORMAT_REGEX = IPRE!();
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

    pub SINGLE_LINE_COMMENT_REGEX = r"^[[:^cntrl:]]*$";

    pub HOSTNAME_REGEX = r"^(?:[a-zA-Z0-9](?:[a-zA-Z0-9\-]*[a-zA-Z0-9])?)$";

    pub DNS_NAME_REGEX =  concat!(r"^", DNS_NAME!(), r"$");

    pub DNS_NAME_OR_IP_REGEX = concat!(r"^", DNS_NAME!(), "|",  IPRE!(), r"$");

    pub PROXMOX_USER_ID_REGEX = concat!(r"^",  USER_NAME_REGEX_STR!(), r"@", PROXMOX_SAFE_ID_REGEX_STR!(), r"$");
}

pub const SYSTEMD_DATETIME_FORMAT: ApiStringFormat =
    ApiStringFormat::Pattern(&SYSTEMD_DATETIME_REGEX);

pub const IP_FORMAT: ApiStringFormat =
    ApiStringFormat::Pattern(&IP_FORMAT_REGEX);

pub const PVE_CONFIG_DIGEST_FORMAT: ApiStringFormat =
    ApiStringFormat::Pattern(&SHA256_HEX_REGEX);

pub const PROXMOX_SAFE_ID_FORMAT: ApiStringFormat =
    ApiStringFormat::Pattern(&PROXMOX_SAFE_ID_REGEX);

pub const SINGLE_LINE_COMMENT_FORMAT: ApiStringFormat =
    ApiStringFormat::Pattern(&SINGLE_LINE_COMMENT_REGEX);

pub const HOSTNAME_FORMAT: ApiStringFormat =
    ApiStringFormat::Pattern(&HOSTNAME_REGEX);

pub const DNS_NAME_FORMAT: ApiStringFormat =
    ApiStringFormat::Pattern(&DNS_NAME_REGEX);

pub const DNS_NAME_OR_IP_FORMAT: ApiStringFormat =
    ApiStringFormat::Pattern(&DNS_NAME_OR_IP_REGEX);

pub const PROXMOX_USER_ID_FORMAT: ApiStringFormat =
    ApiStringFormat::Pattern(&PROXMOX_USER_ID_REGEX);

pub const PASSWORD_FORMAT: ApiStringFormat =
    ApiStringFormat::Pattern(&PASSWORD_REGEX);


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
    .min_length(3)
    .max_length(32)
    .schema();

pub const REMOTE_ID_SCHEMA: Schema = StringSchema::new("Remote ID.")
    .format(&PROXMOX_SAFE_ID_FORMAT)
    .min_length(3)
    .max_length(32)
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

pub const PROXMOX_AUTH_REALM_SCHEMA: Schema = StringSchema::new("Authentication domain ID")
    .format(&PROXMOX_SAFE_ID_FORMAT)
    .min_length(3)
    .max_length(32)
    .schema();

pub const PROXMOX_USER_ID_SCHEMA: Schema = StringSchema::new("User ID")
    .format(&PROXMOX_USER_ID_FORMAT)
    .min_length(3)
    .max_length(64)
    .schema();


// Complex type definitions

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
        files: {
            items: {
                schema: BACKUP_ARCHIVE_NAME_SCHEMA
            },
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
    /// List of contained archive files.
    pub files: Vec<String>,
    /// Overall snapshot size (sum of all archive sizes).
    #[serde(skip_serializing_if="Option::is_none")]
    pub size: Option<u64>,
}


// Regression tests

#[test]
fn test_proxmox_user_id_schema() -> Result<(), Error> {

    let schema = PROXMOX_USER_ID_SCHEMA;

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
        if let Ok(_) = parse_simple_value(name, &schema) {
            bail!("test userid '{}' failed -  got Ok() while expection an error.", name);
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
        let v = match parse_simple_value(name, &schema) {
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
