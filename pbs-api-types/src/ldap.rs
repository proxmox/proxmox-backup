use serde::{Deserialize, Serialize};

use proxmox_schema::{api, ApiStringFormat, ApiType, ArraySchema, Schema, StringSchema, Updater};

use super::{REALM_ID_SCHEMA, SINGLE_LINE_COMMENT_SCHEMA};

#[api()]
#[derive(Copy, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
/// LDAP connection type
pub enum LdapMode {
    /// Plaintext LDAP connection
    #[serde(rename = "ldap")]
    #[default]
    Ldap,
    /// Secure STARTTLS connection
    #[serde(rename = "ldap+starttls")]
    StartTls,
    /// Secure LDAPS connection
    #[serde(rename = "ldaps")]
    Ldaps,
}

#[api(
    properties: {
        "realm": {
            schema: REALM_ID_SCHEMA,
        },
        "comment": {
            optional: true,
            schema: SINGLE_LINE_COMMENT_SCHEMA,
        },
        "verify": {
            optional: true,
            default: false,
        },
        "sync-defaults-options": {
            schema: SYNC_DEFAULTS_STRING_SCHEMA,
            optional: true,
        },
        "sync-attributes": {
            schema: SYNC_ATTRIBUTES_SCHEMA,
            optional: true,
        },
        "user-classes" : {
            optional: true,
            schema: USER_CLASSES_SCHEMA,
        },
        "base-dn" : {
            schema: LDAP_DOMAIN_SCHEMA,
        },
        "bind-dn" : {
            schema: LDAP_DOMAIN_SCHEMA,
            optional: true,
        }
    },
)]
#[derive(Serialize, Deserialize, Updater, Clone)]
#[serde(rename_all = "kebab-case")]
/// LDAP configuration properties.
pub struct LdapRealmConfig {
    #[updater(skip)]
    pub realm: String,
    /// LDAP server address
    pub server1: String,
    /// Fallback LDAP server address
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server2: Option<String>,
    /// Port
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    /// Base domain name. Users are searched under this domain using a `subtree search`.
    pub base_dn: String,
    /// Username attribute. Used to map a ``userid`` to LDAP to an LDAP ``dn``.
    pub user_attr: String,
    /// Comment
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    /// Connection security
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<LdapMode>,
    /// Verify server certificate
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verify: Option<bool>,
    /// CA certificate to use for the server. The path can point to
    /// either a file, or a directory. If it points to a file,
    /// the PEM-formatted X.509 certificate stored at the path
    /// will be added as a trusted certificate.
    /// If the path points to a directory,
    /// the directory replaces the system's default certificate
    /// store at `/etc/ssl/certs` - Every file in the directory
    /// will be loaded as a trusted certificate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capath: Option<String>,
    /// Bind domain to use for looking up users
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bind_dn: Option<String>,
    /// Custom LDAP search filter for user sync
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<String>,
    /// Default options for LDAP sync
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sync_defaults_options: Option<String>,
    /// List of attributes to sync from LDAP to user config
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sync_attributes: Option<String>,
    /// User ``objectClass`` classes to sync
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_classes: Option<String>,
}

#[api(
    properties: {
        "remove-vanished": {
            optional: true,
            schema: REMOVE_VANISHED_SCHEMA,
        },
    },

)]
#[derive(Serialize, Deserialize, Updater, Default, Debug)]
#[serde(rename_all = "kebab-case")]
/// Default options for LDAP synchronization runs
pub struct SyncDefaultsOptions {
    /// How to handle vanished properties/users
    pub remove_vanished: Option<String>,
    /// Enable new users after sync
    pub enable_new: Option<bool>,
}

#[api()]
#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
/// remove-vanished options
pub enum RemoveVanished {
    /// Delete ACLs for vanished users
    Acl,
    /// Remove vanished users
    Entry,
    /// Remove vanished properties from users (e.g. email)
    Properties,
}

pub const LDAP_DOMAIN_SCHEMA: Schema = StringSchema::new("LDAP Domain").schema();

pub const SYNC_DEFAULTS_STRING_SCHEMA: Schema = StringSchema::new("sync defaults options")
    .format(&ApiStringFormat::PropertyString(
        &SyncDefaultsOptions::API_SCHEMA,
    ))
    .schema();

const REMOVE_VANISHED_DESCRIPTION: &str =
    "A semicolon-seperated list of things to remove when they or the user \
vanishes during user synchronization. The following values are possible: ``entry`` removes the \
user when not returned from the sync; ``properties`` removes any  \
properties on existing user that do not appear in the source. \
``acl`` removes ACLs when the user is not returned from the sync.";

pub const REMOVE_VANISHED_SCHEMA: Schema = StringSchema::new(REMOVE_VANISHED_DESCRIPTION)
    .format(&ApiStringFormat::PropertyString(&REMOVE_VANISHED_ARRAY))
    .schema();

pub const REMOVE_VANISHED_ARRAY: Schema = ArraySchema::new(
    "Array of remove-vanished options",
    &RemoveVanished::API_SCHEMA,
)
.min_length(1)
.schema();

#[api()]
#[derive(Serialize, Deserialize, Updater, Default, Debug)]
#[serde(rename_all = "kebab-case")]
/// Determine which LDAP attributes should be synced to which user attributes
pub struct SyncAttributes {
    /// Name of the LDAP attribute containing the user's email address
    pub email: Option<String>,
    /// Name of the LDAP attribute containing the user's first name
    pub firstname: Option<String>,
    /// Name of the LDAP attribute containing the user's last name
    pub lastname: Option<String>,
}

const SYNC_ATTRIBUTES_TEXT: &str = "Comma-separated list of key=value pairs for specifying \
which LDAP attributes map to which PBS user field. For example, \
to map the LDAP attribute ``mail`` to PBS's ``email``, write \
``email=mail``.";

pub const SYNC_ATTRIBUTES_SCHEMA: Schema = StringSchema::new(SYNC_ATTRIBUTES_TEXT)
    .format(&ApiStringFormat::PropertyString(
        &SyncAttributes::API_SCHEMA,
    ))
    .schema();

pub const USER_CLASSES_ARRAY: Schema = ArraySchema::new(
    "Array of user classes",
    &StringSchema::new("user class").schema(),
)
.min_length(1)
.schema();

const USER_CLASSES_TEXT: &str = "Comma-separated list of allowed objectClass values for \
user synchronization. For instance, if ``user-classes`` is set to ``person,user``, \
then user synchronization will consider all LDAP entities \
where ``objectClass: person`` `or` ``objectClass: user``.";

pub const USER_CLASSES_SCHEMA: Schema = StringSchema::new(USER_CLASSES_TEXT)
    .format(&ApiStringFormat::PropertyString(&USER_CLASSES_ARRAY))
    .default("inetorgperson,posixaccount,person,user")
    .schema();
