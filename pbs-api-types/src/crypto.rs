use std::fmt::{self, Display};

use anyhow::Error;
use serde::{Deserialize, Serialize};

use proxmox::api::api;

use pbs_tools::format::{as_fingerprint, bytes_as_fingerprint};

#[api(default: "encrypt")]
#[derive(Copy, Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
/// Defines whether data is encrypted (using an AEAD cipher), only signed, or neither.
pub enum CryptMode {
    /// Don't encrypt.
    None,
    /// Encrypt.
    Encrypt,
    /// Only sign.
    SignOnly,
}

#[derive(Debug, Eq, PartialEq, Hash, Clone, Deserialize, Serialize)]
#[serde(transparent)]
/// 32-byte fingerprint, usually calculated with SHA256.
pub struct Fingerprint {
    #[serde(with = "bytes_as_fingerprint")]
    bytes: [u8; 32],
}

impl Fingerprint {
    pub fn new(bytes: [u8; 32]) -> Self {
        Self { bytes }
    }
    pub fn bytes(&self) -> &[u8; 32] {
        &self.bytes
    }
}

/// Display as short key ID
impl Display for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", as_fingerprint(&self.bytes[0..8]))
    }
}

impl std::str::FromStr for Fingerprint {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Error> {
        let mut tmp = s.to_string();
        tmp.retain(|c| c != ':');
        let bytes = proxmox::tools::hex_to_digest(&tmp)?;
        Ok(Fingerprint::new(bytes))
    }
}

