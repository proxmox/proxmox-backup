use std::path::Path;

use anyhow::{bail, format_err, Error};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use pbs_api_types::{BackupType, CryptMode, Fingerprint};
use pbs_tools::crypt_config::CryptConfig;

pub const MANIFEST_BLOB_NAME: &str = "index.json.blob";
pub const MANIFEST_LOCK_NAME: &str = ".index.json.lck";
pub const CLIENT_LOG_BLOB_NAME: &str = "client.log.blob";
pub const ENCRYPTED_KEY_BLOB_NAME: &str = "rsa-encrypted.key.blob";

fn crypt_mode_none() -> CryptMode {
    CryptMode::None
}
fn empty_value() -> Value {
    json!({})
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct FileInfo {
    pub filename: String,
    #[serde(default = "crypt_mode_none")] // to be compatible with < 0.8.0 backups
    pub crypt_mode: CryptMode,
    pub size: u64,
    #[serde(with = "hex::serde")]
    pub csum: [u8; 32],
}

impl FileInfo {
    /// Return expected CryptMode of referenced chunks
    ///
    /// Encrypted Indices should only reference encrypted chunks, while signed or plain indices
    /// should only reference plain chunks.
    pub fn chunk_crypt_mode(&self) -> CryptMode {
        match self.crypt_mode {
            CryptMode::Encrypt => CryptMode::Encrypt,
            CryptMode::SignOnly | CryptMode::None => CryptMode::None,
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct BackupManifest {
    backup_type: BackupType,
    backup_id: String,
    backup_time: i64,
    files: Vec<FileInfo>,
    #[serde(default = "empty_value")] // to be compatible with < 0.8.0 backups
    pub unprotected: Value,
    pub signature: Option<String>,
}

#[derive(PartialEq, Eq)]
pub enum ArchiveType {
    FixedIndex,
    DynamicIndex,
    Blob,
}

impl ArchiveType {
    pub fn from_path(archive_name: impl AsRef<Path>) -> Result<Self, Error> {
        let archive_name = archive_name.as_ref();
        let archive_type = match archive_name.extension().and_then(|ext| ext.to_str()) {
            Some("didx") => ArchiveType::DynamicIndex,
            Some("fidx") => ArchiveType::FixedIndex,
            Some("blob") => ArchiveType::Blob,
            _ => bail!("unknown archive type: {:?}", archive_name),
        };
        Ok(archive_type)
    }
}

//#[deprecated(note = "use ArchivType::from_path instead")] later...
pub fn archive_type<P: AsRef<Path>>(archive_name: P) -> Result<ArchiveType, Error> {
    ArchiveType::from_path(archive_name)
}

impl BackupManifest {
    pub fn new(snapshot: pbs_api_types::BackupDir) -> Self {
        Self {
            backup_type: snapshot.group.ty,
            backup_id: snapshot.group.id,
            backup_time: snapshot.time,
            files: Vec::new(),
            unprotected: json!({}),
            signature: None,
        }
    }

    pub fn add_file(
        &mut self,
        filename: String,
        size: u64,
        csum: [u8; 32],
        crypt_mode: CryptMode,
    ) -> Result<(), Error> {
        let _archive_type = ArchiveType::from_path(&filename)?; // check type
        self.files.push(FileInfo {
            filename,
            size,
            csum,
            crypt_mode,
        });
        Ok(())
    }

    pub fn files(&self) -> &[FileInfo] {
        &self.files[..]
    }

    pub fn lookup_file_info(&self, name: &str) -> Result<&FileInfo, Error> {
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

    // Generate canonical json
    fn to_canonical_json(value: &Value) -> Result<Vec<u8>, Error> {
        proxmox_serde::json::to_canonical_json(value)
    }

    /// Compute manifest signature
    ///
    /// By generating a HMAC SHA256 over the canonical json
    /// representation, The 'unpreotected' property is excluded.
    pub fn signature(&self, crypt_config: &CryptConfig) -> Result<[u8; 32], Error> {
        Self::json_signature(&serde_json::to_value(self)?, crypt_config)
    }

    fn json_signature(data: &Value, crypt_config: &CryptConfig) -> Result<[u8; 32], Error> {
        let mut signed_data = data.clone();

        signed_data.as_object_mut().unwrap().remove("unprotected"); // exclude
        signed_data.as_object_mut().unwrap().remove("signature"); // exclude

        let canonical = Self::to_canonical_json(&signed_data)?;

        let sig = crypt_config.compute_auth_tag(&canonical);

        Ok(sig)
    }

    /// Converts the Manifest into json string, and add a signature if there is a crypt_config.
    pub fn to_string(&self, crypt_config: Option<&CryptConfig>) -> Result<String, Error> {
        let mut manifest = serde_json::to_value(self)?;

        if let Some(crypt_config) = crypt_config {
            let sig = self.signature(crypt_config)?;
            manifest["signature"] = hex::encode(sig).into();
            let fingerprint = &Fingerprint::new(crypt_config.fingerprint());
            manifest["unprotected"]["key-fingerprint"] = serde_json::to_value(fingerprint)?;
        }

        let manifest = serde_json::to_string_pretty(&manifest).unwrap();
        Ok(manifest)
    }

    pub fn fingerprint(&self) -> Result<Option<Fingerprint>, Error> {
        match &self.unprotected["key-fingerprint"] {
            Value::Null => Ok(None),
            value => Ok(Some(Deserialize::deserialize(value)?)),
        }
    }

    /// Checks if a BackupManifest and a CryptConfig share a valid fingerprint combination.
    ///
    /// An unsigned manifest is valid with any or no CryptConfig.
    /// A signed manifest is only valid with a matching CryptConfig.
    pub fn check_fingerprint(&self, crypt_config: Option<&CryptConfig>) -> Result<(), Error> {
        if let Some(fingerprint) = self.fingerprint()? {
            match crypt_config {
                None => bail!(
                    "missing key - manifest was created with key {}",
                    fingerprint,
                ),
                Some(crypt_config) => {
                    let config_fp = Fingerprint::new(crypt_config.fingerprint());
                    if config_fp != fingerprint {
                        bail!(
                            "wrong key - manifest's key {} does not match provided key {}",
                            fingerprint,
                            config_fp
                        );
                    }
                }
            }
        };

        Ok(())
    }

    /// Try to read the manifest. This verifies the signature if there is a crypt_config.
    pub fn from_data(
        data: &[u8],
        crypt_config: Option<&CryptConfig>,
    ) -> Result<BackupManifest, Error> {
        let json: Value = serde_json::from_slice(data)?;
        let signature = json["signature"].as_str().map(String::from);

        if let Some(crypt_config) = crypt_config {
            if let Some(signature) = signature {
                let expected_signature = hex::encode(Self::json_signature(&json, crypt_config)?);

                let fingerprint = &json["unprotected"]["key-fingerprint"];
                if fingerprint != &Value::Null {
                    let fingerprint = Fingerprint::deserialize(fingerprint)?;
                    let config_fp = Fingerprint::new(crypt_config.fingerprint());
                    if config_fp != fingerprint {
                        bail!(
                            "wrong key - unable to verify signature since manifest's key {} does not match provided key {}",
                            fingerprint,
                            config_fp
                        );
                    }
                }
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
        // no expected digest available
        let data = blob
            .decode(None, None)
            .map_err(|err| format_err!("decode backup manifest blob failed - {}", err))?;
        let json: Value = serde_json::from_slice(&data[..])
            .map_err(|err| format_err!("unable to parse backup manifest json - {}", err))?;
        let manifest: BackupManifest = serde_json::from_value(json)?;
        Ok(manifest)
    }
}

#[test]
fn test_manifest_signature() -> Result<(), Error> {
    use pbs_key_config::KeyDerivationConfig;

    let pw = b"test";

    let kdf = KeyDerivationConfig::Scrypt {
        n: 65536,
        r: 8,
        p: 1,
        salt: Vec::new(),
    };

    let testkey = kdf.derive_key(pw)?;

    let crypt_config = CryptConfig::new(testkey)?;

    let mut manifest = BackupManifest::new("host/elsa/2020-06-26T13:56:05Z".parse()?);

    manifest.add_file("test1.img.fidx".into(), 200, [1u8; 32], CryptMode::Encrypt)?;
    manifest.add_file("abc.blob".into(), 200, [2u8; 32], CryptMode::None)?;

    manifest.unprotected["note"] = "This is not protected by the signature.".into();

    let text = manifest.to_string(Some(&crypt_config))?;

    let manifest: Value = serde_json::from_str(&text)?;
    let signature = manifest["signature"].as_str().unwrap().to_string();

    assert_eq!(
        signature,
        "d7b446fb7db081662081d4b40fedd858a1d6307a5aff4ecff7d5bf4fd35679e9"
    );

    let manifest: BackupManifest = serde_json::from_value(manifest)?;
    let expected_signature = hex::encode(manifest.signature(&crypt_config)?);

    assert_eq!(signature, expected_signature);

    Ok(())
}
