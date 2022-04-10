use serde::{Deserialize, Serialize};

use proxmox_schema::api;

use crate::CERT_FINGERPRINT_SHA256_SCHEMA;

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub kdf: Kdf,
    /// Key creation time
    pub created: i64,
    /// Key modification time
    pub modified: i64,
    /// Key fingerprint
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
    /// Password hint
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}
