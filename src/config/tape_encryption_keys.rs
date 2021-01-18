use std::collections::HashMap;

use anyhow::{bail, Error};
use serde::{Deserialize, Serialize};
use openssl::sha::sha256;

use proxmox::tools::fs::{
    file_read_optional_string,
    replace_file,
    CreateOptions,
};

use crate::{
    backup::{
        Fingerprint,
    },
};

mod hex_key {
    use serde::{self, Deserialize, Serializer, Deserializer};

    pub fn serialize<S>(
        csum: &[u8; 32],
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let s = proxmox::tools::digest_to_hex(csum);
        serializer.serialize_str(&s)
    }

    pub fn deserialize<'de, D>(
        deserializer: D,
    ) -> Result<[u8; 32], D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        proxmox::tools::hex_to_digest(&s).map_err(serde::de::Error::custom)
    }
}

/// Store Hardware Encryption keys
#[derive(Deserialize, Serialize)]
pub struct EncryptionKeyInfo {
    pub hint: String,
    #[serde(with = "hex_key")]
    pub key: [u8; 32],
    pub fingerprint: Fingerprint,
}

impl EncryptionKeyInfo {

    pub fn new(key: &[u8; 32], hint: String) -> Self {
        Self {
            hint,
            key: key.clone(),
            fingerprint: Fingerprint::new(sha256(key)),
        }
    }
}

pub const TAPE_KEYS_FILENAME: &str = "/etc/proxmox-backup/tape-encryption-keys.json";
pub const TAPE_KEYS_LOCKFILE: &str = "/etc/proxmox-backup/.tape-encryption-keys.lck";

pub fn load_keys() -> Result<(HashMap<Fingerprint, EncryptionKeyInfo>,  [u8;32]), Error> {

    let content = file_read_optional_string(TAPE_KEYS_FILENAME)?;
    let content = content.unwrap_or_else(|| String::from("[]"));

    let digest = openssl::sha::sha256(content.as_bytes());

    let list: Vec<EncryptionKeyInfo> = serde_json::from_str(&content)?;

    let mut map = HashMap::new();
    
    for item in list {
        let expected_fingerprint = Fingerprint::new(sha256(&item.key));
        if item.fingerprint != expected_fingerprint {
            bail!(
                "inconsistent fingerprint ({} != {})",
                item.fingerprint,
                expected_fingerprint,
            );
        }
        
        map.insert(item.fingerprint.clone(), item);
    }
   
    Ok((map, digest))
}

pub fn save_keys(map: HashMap<Fingerprint, EncryptionKeyInfo>) -> Result<(), Error> {

    let mut list = Vec::new();

    for (_fp, item) in map {
        list.push(item);
    }

    let raw = serde_json::to_string_pretty(&list)?;
    
    let mode = nix::sys::stat::Mode::from_bits_truncate(0o0600);
    // set the correct owner/group/permissions while saving file
    // owner(rw) = root, group(r)= root
    let options = CreateOptions::new()
        .perm(mode)
        .owner(nix::unistd::ROOT)
        .group(nix::unistd::Gid::from_raw(0));

    replace_file(TAPE_KEYS_FILENAME, raw.as_bytes(), options)?;
    
    Ok(())
}

// shell completion helper
pub fn complete_key_fingerprint(_arg: &str, _param: &HashMap<String, String>) -> Vec<String> {
    let data = match load_keys() {
        Ok((data, _digest)) => data,
        Err(_) => return Vec::new(),
    };

    data.keys().map(|fp| crate::tools::format::as_fingerprint(fp.bytes())).collect()
}

