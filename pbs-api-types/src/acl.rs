use std::str::FromStr;

use serde::de::{value, IntoDeserializer};
use serde::{Deserialize, Serialize};

use proxmox_lang::constnamedbitmap;
use proxmox_schema::{
    api, const_regex, ApiStringFormat, BooleanSchema, EnumEntry, Schema, StringSchema,
};

const_regex! {
    pub ACL_PATH_REGEX = concat!(r"^(?:/|", r"(?:/", PROXMOX_SAFE_ID_REGEX_STR!(), ")+", r")$");
}

// define Privilege bitfield

constnamedbitmap! {
    /// Contains a list of privilege name to privilege value mappings.
    ///
    /// The names are used when displaying/persisting privileges anywhere, the values are used to
    /// allow easy matching of privileges as bitflags.
    PRIVILEGES: u64 => {
        /// Sys.Audit allows knowing about the system and its status
        PRIV_SYS_AUDIT("Sys.Audit");
        /// Sys.Modify allows modifying system-level configuration
        PRIV_SYS_MODIFY("Sys.Modify");
        /// Sys.Modify allows to poweroff/reboot/.. the system
        PRIV_SYS_POWER_MANAGEMENT("Sys.PowerManagement");

        /// Datastore.Audit allows knowing about a datastore,
        /// including reading the configuration entry and listing its contents
        PRIV_DATASTORE_AUDIT("Datastore.Audit");
        /// Datastore.Allocate allows creating or deleting datastores
        PRIV_DATASTORE_ALLOCATE("Datastore.Allocate");
        /// Datastore.Modify allows modifying a datastore and its contents
        PRIV_DATASTORE_MODIFY("Datastore.Modify");
        /// Datastore.Read allows reading arbitrary backup contents
        PRIV_DATASTORE_READ("Datastore.Read");
        /// Allows verifying a datastore
        PRIV_DATASTORE_VERIFY("Datastore.Verify");

        /// Datastore.Backup allows Datastore.Read|Verify and creating new snapshots,
        /// but also requires backup ownership
        PRIV_DATASTORE_BACKUP("Datastore.Backup");
        /// Datastore.Prune allows deleting snapshots,
        /// but also requires backup ownership
        PRIV_DATASTORE_PRUNE("Datastore.Prune");

        /// Permissions.Modify allows modifying ACLs
        PRIV_PERMISSIONS_MODIFY("Permissions.Modify");

        /// Remote.Audit allows reading remote.cfg and sync.cfg entries
        PRIV_REMOTE_AUDIT("Remote.Audit");
        /// Remote.Modify allows modifying remote.cfg
        PRIV_REMOTE_MODIFY("Remote.Modify");
        /// Remote.Read allows reading data from a configured `Remote`
        PRIV_REMOTE_READ("Remote.Read");

        /// Sys.Console allows access to the system's console
        PRIV_SYS_CONSOLE("Sys.Console");

        /// Tape.Audit allows reading tape backup configuration and status
        PRIV_TAPE_AUDIT("Tape.Audit");
        /// Tape.Modify allows modifying tape backup configuration
        PRIV_TAPE_MODIFY("Tape.Modify");
        /// Tape.Write allows writing tape media
        PRIV_TAPE_WRITE("Tape.Write");
        /// Tape.Read allows reading tape backup configuration and media contents
        PRIV_TAPE_READ("Tape.Read");

        /// Realm.Allocate allows viewing, creating, modifying and deleting realms
        PRIV_REALM_ALLOCATE("Realm.Allocate");
    }
}

pub fn privs_to_priv_names(privs: u64) -> Vec<&'static str> {
    PRIVILEGES
        .iter()
        .fold(Vec::new(), |mut priv_names, (name, value)| {
            if value & privs != 0 {
                priv_names.push(name);
            }
            priv_names
        })
}

/// Admin always has all privileges. It can do everything except a few actions
/// which are limited to the 'root@pam` superuser
pub const ROLE_ADMIN: u64 = u64::MAX;

/// NoAccess can be used to remove privileges from specific (sub-)paths
pub const ROLE_NO_ACCESS: u64 = 0;

#[rustfmt::skip]
#[allow(clippy::identity_op)]
/// Audit can view configuration and status information, but not modify it.
pub const ROLE_AUDIT: u64 = 0
    | PRIV_SYS_AUDIT
    | PRIV_DATASTORE_AUDIT;

#[rustfmt::skip]
#[allow(clippy::identity_op)]
/// Datastore.Admin can do anything on the datastore.
pub const ROLE_DATASTORE_ADMIN: u64 = 0
    | PRIV_DATASTORE_AUDIT
    | PRIV_DATASTORE_MODIFY
    | PRIV_DATASTORE_READ
    | PRIV_DATASTORE_VERIFY
    | PRIV_DATASTORE_BACKUP
    | PRIV_DATASTORE_PRUNE;

#[rustfmt::skip]
#[allow(clippy::identity_op)]
/// Datastore.Reader can read/verify datastore content and do restore
pub const ROLE_DATASTORE_READER: u64 = 0
    | PRIV_DATASTORE_AUDIT
    | PRIV_DATASTORE_VERIFY
    | PRIV_DATASTORE_READ;

#[rustfmt::skip]
#[allow(clippy::identity_op)]
/// Datastore.Backup can do backup and restore, but no prune.
pub const ROLE_DATASTORE_BACKUP: u64 = 0
    | PRIV_DATASTORE_BACKUP;

#[rustfmt::skip]
#[allow(clippy::identity_op)]
/// Datastore.PowerUser can do backup, restore, and prune.
pub const ROLE_DATASTORE_POWERUSER: u64 = 0
    | PRIV_DATASTORE_PRUNE
    | PRIV_DATASTORE_BACKUP;

#[rustfmt::skip]
#[allow(clippy::identity_op)]
/// Datastore.Audit can audit the datastore.
pub const ROLE_DATASTORE_AUDIT: u64 = 0
    | PRIV_DATASTORE_AUDIT;

#[rustfmt::skip]
#[allow(clippy::identity_op)]
/// Remote.Audit can audit the remote
pub const ROLE_REMOTE_AUDIT: u64 = 0
    | PRIV_REMOTE_AUDIT;

#[rustfmt::skip]
#[allow(clippy::identity_op)]
/// Remote.Admin can do anything on the remote.
pub const ROLE_REMOTE_ADMIN: u64 = 0
    | PRIV_REMOTE_AUDIT
    | PRIV_REMOTE_MODIFY
    | PRIV_REMOTE_READ;

#[rustfmt::skip]
#[allow(clippy::identity_op)]
/// Remote.SyncOperator can do read and prune on the remote.
pub const ROLE_REMOTE_SYNC_OPERATOR: u64 = 0
    | PRIV_REMOTE_AUDIT
    | PRIV_REMOTE_READ;

#[rustfmt::skip]
#[allow(clippy::identity_op)]
/// Tape.Audit can audit the tape backup configuration and media content
pub const ROLE_TAPE_AUDIT: u64 = 0
    | PRIV_TAPE_AUDIT;

#[rustfmt::skip]
#[allow(clippy::identity_op)]
/// Tape.Admin can do anything on the tape backup
pub const ROLE_TAPE_ADMIN: u64 = 0
    | PRIV_TAPE_AUDIT
    | PRIV_TAPE_MODIFY
    | PRIV_TAPE_READ
    | PRIV_TAPE_WRITE;

#[rustfmt::skip]
#[allow(clippy::identity_op)]
/// Tape.Operator can do tape backup and restore (but no configuration changes)
pub const ROLE_TAPE_OPERATOR: u64 = 0
    | PRIV_TAPE_AUDIT
    | PRIV_TAPE_READ
    | PRIV_TAPE_WRITE;

#[rustfmt::skip]
#[allow(clippy::identity_op)]
/// Tape.Reader can do read and inspect tape content
pub const ROLE_TAPE_READER: u64 = 0
    | PRIV_TAPE_AUDIT
    | PRIV_TAPE_READ;

/// NoAccess can be used to remove privileges from specific (sub-)paths
pub const ROLE_NAME_NO_ACCESS: &str = "NoAccess";

#[api(
    type_text: "<role>",
)]
#[repr(u64)]
#[derive(Serialize, Deserialize)]
/// Enum representing roles via their [PRIVILEGES] combination.
///
/// Since privileges are implemented as bitflags, each unique combination of privileges maps to a
/// single, unique `u64` value that is used in this enum definition.
pub enum Role {
    /// Administrator
    Admin = ROLE_ADMIN,
    /// Auditor
    Audit = ROLE_AUDIT,
    /// Disable Access
    NoAccess = ROLE_NO_ACCESS,
    /// Datastore Administrator
    DatastoreAdmin = ROLE_DATASTORE_ADMIN,
    /// Datastore Reader (inspect datastore content and do restores)
    DatastoreReader = ROLE_DATASTORE_READER,
    /// Datastore Backup (backup and restore owned backups)
    DatastoreBackup = ROLE_DATASTORE_BACKUP,
    /// Datastore PowerUser (backup, restore and prune owned backup)
    DatastorePowerUser = ROLE_DATASTORE_POWERUSER,
    /// Datastore Auditor
    DatastoreAudit = ROLE_DATASTORE_AUDIT,
    /// Remote Auditor
    RemoteAudit = ROLE_REMOTE_AUDIT,
    /// Remote Administrator
    RemoteAdmin = ROLE_REMOTE_ADMIN,
    /// Syncronisation Opertator
    RemoteSyncOperator = ROLE_REMOTE_SYNC_OPERATOR,
    /// Tape Auditor
    TapeAudit = ROLE_TAPE_AUDIT,
    /// Tape Administrator
    TapeAdmin = ROLE_TAPE_ADMIN,
    /// Tape Operator
    TapeOperator = ROLE_TAPE_OPERATOR,
    /// Tape Reader
    TapeReader = ROLE_TAPE_READER,
}

impl FromStr for Role {
    type Err = value::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::deserialize(s.into_deserializer())
    }
}

pub const ACL_PATH_FORMAT: ApiStringFormat = ApiStringFormat::Pattern(&ACL_PATH_REGEX);

pub const ACL_PATH_SCHEMA: Schema = StringSchema::new("Access control path.")
    .format(&ACL_PATH_FORMAT)
    .min_length(1)
    .max_length(128)
    .schema();

pub const ACL_PROPAGATE_SCHEMA: Schema =
    BooleanSchema::new("Allow to propagate (inherit) permissions.")
        .default(true)
        .schema();

pub const ACL_UGID_TYPE_SCHEMA: Schema = StringSchema::new("Type of 'ugid' property.")
    .format(&ApiStringFormat::Enum(&[
        EnumEntry::new("user", "User"),
        EnumEntry::new("group", "Group"),
    ]))
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
#[derive(Serialize, Deserialize, Clone, PartialEq)]
/// ACL list entry.
pub struct AclListItem {
    pub path: String,
    pub ugid: String,
    pub ugid_type: String,
    pub propagate: bool,
    pub roleid: String,
}
