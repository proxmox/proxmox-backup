use anyhow::{bail, Error};

use proxmox_schema::{ApiStringFormat, Schema, StringSchema};

use crate::{CHANGER_NAME_SCHEMA, PROXMOX_SAFE_ID_FORMAT};

pub const VAULT_NAME_SCHEMA: Schema = StringSchema::new("Vault name.")
    .format(&PROXMOX_SAFE_ID_FORMAT)
    .min_length(3)
    .max_length(32)
    .schema();

#[derive(Debug, PartialEq, Eq, Clone)]
/// Media location
pub enum MediaLocation {
    /// Ready for use (inside tape library)
    Online(String),
    /// Local available, but need to be mounted (insert into tape
    /// drive)
    Offline,
    /// Media is inside a Vault
    Vault(String),
}

proxmox_serde::forward_deserialize_to_from_str!(MediaLocation);
proxmox_serde::forward_serialize_to_display!(MediaLocation);

impl proxmox_schema::ApiType for MediaLocation {
    const API_SCHEMA: Schema = StringSchema::new(
        "Media location (e.g. 'offline', 'online-<changer_name>', 'vault-<vault_name>')",
    )
    .format(&ApiStringFormat::VerifyFn(|text| {
        let location: MediaLocation = text.parse()?;
        match location {
            MediaLocation::Online(ref changer) => {
                CHANGER_NAME_SCHEMA.parse_simple_value(changer)?;
            }
            MediaLocation::Vault(ref vault) => {
                VAULT_NAME_SCHEMA.parse_simple_value(vault)?;
            }
            MediaLocation::Offline => { /* OK */ }
        }
        Ok(())
    }))
    .schema();
}

impl std::fmt::Display for MediaLocation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MediaLocation::Offline => {
                write!(f, "offline")
            }
            MediaLocation::Online(changer) => {
                write!(f, "online-{}", changer)
            }
            MediaLocation::Vault(vault) => {
                write!(f, "vault-{}", vault)
            }
        }
    }
}

impl std::str::FromStr for MediaLocation {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s == "offline" {
            return Ok(MediaLocation::Offline);
        }
        if let Some(changer) = s.strip_prefix("online-") {
            return Ok(MediaLocation::Online(changer.to_string()));
        }
        if let Some(vault) = s.strip_prefix("vault-") {
            return Ok(MediaLocation::Vault(vault.to_string()));
        }

        bail!("MediaLocation parse error");
    }
}
