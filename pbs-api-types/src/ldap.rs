use serde::{Deserialize, Serialize};

use proxmox_schema::{api, Updater};

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
}
