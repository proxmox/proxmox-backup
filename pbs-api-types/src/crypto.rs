use std::fmt::{self, Display};

use anyhow::Error;
use serde::{Deserialize, Serialize};

use proxmox_schema::api;

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
    pub fn signature(&self) -> String {
        as_fingerprint(&self.bytes)
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
        let mut bytes = [0u8; 32];
        hex::decode_to_slice(&tmp, &mut bytes)?;
        Ok(Fingerprint::new(bytes))
    }
}

fn as_fingerprint(bytes: &[u8]) -> String {
    hex::encode(bytes)
        .as_bytes()
        .chunks(2)
        .map(|v| unsafe { std::str::from_utf8_unchecked(v) }) // it's a hex string
        .collect::<Vec<&str>>()
        .join(":")
}

pub mod bytes_as_fingerprint {
    use std::mem::MaybeUninit;

    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(bytes: &[u8; 32], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let s = super::as_fingerprint(bytes);
        serializer.serialize_str(&s)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<[u8; 32], D::Error>
    where
        D: Deserializer<'de>,
    {
        // TODO: more efficiently implement with a Visitor implementing visit_str using split() and
        // hex::decode by-byte
        let mut s = String::deserialize(deserializer)?;
        s.retain(|c| c != ':');
        let mut out = MaybeUninit::<[u8; 32]>::uninit();
        hex::decode_to_slice(s.as_bytes(), unsafe { &mut (*out.as_mut_ptr())[..] })
            .map_err(serde::de::Error::custom)?;
        Ok(unsafe { out.assume_init() })
    }
}
