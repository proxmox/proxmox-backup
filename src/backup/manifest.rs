use anyhow::{bail, format_err, Error};
use std::convert::TryFrom;
use std::path::Path;

use serde_json::{json, Value};
use ::serde::{Deserialize, Serialize};

use crate::backup::{BackupDir, CryptMode, CryptConfig};

pub const MANIFEST_BLOB_NAME: &str = "index.json.blob";
pub const CLIENT_LOG_BLOB_NAME: &str = "client.log.blob";

mod hex_csum {
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

fn crypt_mode_none() -> CryptMode { CryptMode::None }
fn empty_value() -> Value { json!({}) }

#[derive(Serialize, Deserialize)]
#[serde(rename_all="kebab-case")]
pub struct FileInfo {
    pub filename: String,
    #[serde(default="crypt_mode_none")] // to be compatible with < 0.8.0 backups
    pub crypt_mode: CryptMode,
    pub size: u64,
    #[serde(with = "hex_csum")]
    pub csum: [u8; 32],
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all="kebab-case")]
pub struct BackupManifest {
    backup_type: String,
    backup_id: String,
    backup_time: i64,
    files: Vec<FileInfo>,
    #[serde(default="empty_value")] // to be compatible with < 0.8.0 backups
    pub unprotected: Value,
}

#[derive(PartialEq)]
pub enum ArchiveType {
    FixedIndex,
    DynamicIndex,
    Blob,
}

pub fn archive_type<P: AsRef<Path>>(
    archive_name: P,
) -> Result<ArchiveType, Error> {

    let archive_name = archive_name.as_ref();
    let archive_type = match archive_name.extension().and_then(|ext| ext.to_str()) {
        Some("didx") => ArchiveType::DynamicIndex,
        Some("fidx") => ArchiveType::FixedIndex,
        Some("blob") => ArchiveType::Blob,
        _ => bail!("unknown archive type: {:?}", archive_name),
    };
    Ok(archive_type)
}


impl BackupManifest {

    pub fn new(snapshot: BackupDir) -> Self {
        Self {
            backup_type: snapshot.group().backup_type().into(),
            backup_id: snapshot.group().backup_id().into(),
            backup_time: snapshot.backup_time().timestamp(),
            files: Vec::new(),
            unprotected: json!({}),
        }
    }

    pub fn add_file(&mut self, filename: String, size: u64, csum: [u8; 32], crypt_mode: CryptMode) -> Result<(), Error> {
        let _archive_type = archive_type(&filename)?; // check type
        self.files.push(FileInfo { filename, size, csum, crypt_mode });
        Ok(())
    }

    pub fn files(&self) -> &[FileInfo] {
        &self.files[..]
    }

    fn lookup_file_info(&self, name: &str) -> Result<&FileInfo, Error> {

        let info = self.files.iter().find(|item| item.filename == name);

        match info {
            None => bail!("manifest does not contain file '{}'", name),
            Some(info) => Ok(info),
        }
    }

    pub fn verify_file(&self, name: &str, csum: &[u8; 32], size: u64) -> Result<(), Error> {

        let info = self.lookup_file_info(name)?;

        if size != info.size {
            bail!("wrong size for file '{}' ({} != {})", name, info.size, size);
        }

        if csum != &info.csum {
            bail!("wrong checksum for file '{}'", name);
        }

        Ok(())
    }

    // Generate cannonical json
    fn to_canonical_json(value: &Value, output: &mut String) -> Result<(), Error> {
        match value {
            Value::Null => bail!("got unexpected null value"),
            Value::String(_) => {
                output.push_str(&serde_json::to_string(value)?);
             },
            Value::Number(_) => {
                output.push_str(&serde_json::to_string(value)?);
            }
            Value::Bool(_) => {
                output.push_str(&serde_json::to_string(value)?);
             },
            Value::Array(list) => {
                output.push('[');
                for (i, item) in list.iter().enumerate() {
                    if i != 0 { output.push(','); }
                    Self::to_canonical_json(item, output)?;
                }
                output.push(']');
              }
            Value::Object(map) => {
                output.push('{');
                let mut keys: Vec<String> = map.keys().map(|s| s.clone()).collect();
                keys.sort();
                for (i, key) in keys.iter().enumerate() {
                    let item = map.get(key).unwrap();
                    if i != 0 { output.push(','); }

                    output.push_str(&serde_json::to_string(&Value::String(key.clone()))?);
                    output.push(':');
                    Self::to_canonical_json(item, output)?;
                }
                output.push('}');
            }
        }
        Ok(())
    }

    /// Compute manifest signature
    ///
    /// By generating a HMAC SHA256 over the canonical json
    /// representation, The 'unpreotected' property is excluded.
    pub fn signature(&self, crypt_config: &CryptConfig) -> Result<[u8; 32], Error> {
        Self::json_signature(&serde_json::to_value(&self)?, crypt_config)
    }

    fn json_signature(data: &Value, crypt_config: &CryptConfig) -> Result<[u8; 32], Error> {

        let mut signed_data = data.clone();

        signed_data.as_object_mut().unwrap().remove("unprotected"); // exclude

        let mut canonical = String::new();
        Self::to_canonical_json(&signed_data, &mut canonical)?;

        let sig = crypt_config.compute_auth_tag(canonical.as_bytes());

        Ok(sig)
    }

    /// Converts the Manifest into json string, and add a signature if there is a crypt_config.
    pub fn to_string(&self, crypt_config: Option<&CryptConfig>) -> Result<String, Error> {

        let mut manifest = serde_json::to_value(&self)?;

        if let Some(crypt_config) = crypt_config {
            let sig = self.signature(crypt_config)?;
            manifest["signature"] = proxmox::tools::digest_to_hex(&sig).into();
        }

        let manifest = serde_json::to_string_pretty(&manifest).unwrap().into();
        Ok(manifest)
    }

    /// Try to read the manifest. This verifies the signature if there is a crypt_config.
    pub fn from_data(data: &[u8], crypt_config: Option<&CryptConfig>) -> Result<BackupManifest, Error> {
        let json: Value = serde_json::from_slice(data)?;
        let signature = json["signature"].as_str().map(String::from);

        if let Some(ref crypt_config) = crypt_config {
            if let Some(signature) = signature {
                let expected_signature = proxmox::tools::digest_to_hex(&Self::json_signature(&json, crypt_config)?);
                if signature != expected_signature {
                    bail!("wrong signature in manifest");
                }
            } else {
                // not signed: warn/fail?
            }
        }

        let manifest: BackupManifest = serde_json::from_value(json)?;
        Ok(manifest)
    }
}


impl TryFrom<super::DataBlob> for BackupManifest {
    type Error = Error;

    fn try_from(blob: super::DataBlob) -> Result<Self, Error> {
        let data = blob.decode(None)
            .map_err(|err| format_err!("decode backup manifest blob failed - {}", err))?;
        let json: Value = serde_json::from_slice(&data[..])
            .map_err(|err| format_err!("unable to parse backup manifest json - {}", err))?;
        let manifest: BackupManifest = serde_json::from_value(json)?;
        Ok(manifest)
    }
}


#[test]
fn test_manifest_signature() -> Result<(), Error> {

    use crate::backup::{KeyDerivationConfig};

    let pw = b"test";

    let kdf = KeyDerivationConfig::Scrypt {
        n: 65536,
        r: 8,
        p: 1,
        salt: Vec::new(),
    };

    let testkey = kdf.derive_key(pw)?;

    let crypt_config = CryptConfig::new(testkey)?;

    let snapshot: BackupDir = "host/elsa/2020-06-26T13:56:05Z".parse()?;

    let mut manifest = BackupManifest::new(snapshot);

    manifest.add_file("test1.img.fidx".into(), 200, [1u8; 32], CryptMode::Encrypt)?;
    manifest.add_file("abc.blob".into(), 200, [2u8; 32], CryptMode::None)?;

    manifest.unprotected["note"] = "This is not protected by the signature.".into();

    let text = manifest.to_string(Some(&crypt_config))?;

    let manifest: Value = serde_json::from_str(&text)?;
    let signature = manifest["signature"].as_str().unwrap().to_string();

    assert_eq!(signature, "d7b446fb7db081662081d4b40fedd858a1d6307a5aff4ecff7d5bf4fd35679e9");

    let manifest: BackupManifest = serde_json::from_value(manifest)?;
    let expected_signature = proxmox::tools::digest_to_hex(&manifest.signature(&crypt_config)?);

    assert_eq!(signature, expected_signature);

    Ok(())
}
