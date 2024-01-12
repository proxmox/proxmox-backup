use serde::{Deserialize, Serialize};

use proxmox_schema::{api, Updater};

use super::{
    LdapMode, LDAP_DOMAIN_SCHEMA, REALM_ID_SCHEMA, SINGLE_LINE_COMMENT_SCHEMA,
    SYNC_ATTRIBUTES_SCHEMA, SYNC_DEFAULTS_STRING_SCHEMA, USER_CLASSES_SCHEMA,
};

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
            optional: true,
        },
        "bind-dn" : {
            schema: LDAP_DOMAIN_SCHEMA,
            optional: true,
        }
    },
)]
#[derive(Serialize, Deserialize, Updater, Clone)]
#[serde(rename_all = "kebab-case")]
/// AD realm configuration properties.
pub struct AdRealmConfig {
    #[updater(skip)]
    pub realm: String,
    /// AD server address
    pub server1: String,
    /// Fallback AD server address
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server2: Option<String>,
    /// AD server Port
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    /// Base domain name. Users are searched under this domain using a `subtree search`.
    /// Expected to be set only internally to `defaultNamingContext` of the AD server, but can be
    /// overridden if the need arises.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_dn: Option<String>,
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
    /// Default options for AD sync
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sync_defaults_options: Option<String>,
    /// List of LDAP attributes to sync from AD to user config
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sync_attributes: Option<String>,
    /// User ``objectClass`` classes to sync
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_classes: Option<String>,
}
