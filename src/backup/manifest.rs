use anyhow::{bail, format_err, Error};
use std::convert::TryFrom;
use std::path::Path;

use serde_json::{json, Value};

use crate::backup::{BackupDir, CryptMode};

pub const MANIFEST_BLOB_NAME: &str = "index.json.blob";
pub const CLIENT_LOG_BLOB_NAME: &str = "client.log.blob";

pub struct FileInfo {
    pub filename: String,
    pub crypt_mode: CryptMode,
    pub size: u64,
    pub csum: [u8; 32],
}

pub struct BackupManifest {
    snapshot: BackupDir,
    files: Vec<FileInfo>,
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
        Self { files: Vec::new(), snapshot }
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

    pub fn into_json(self) -> Value {
        json!({
            "backup-type": self.snapshot.group().backup_type(),
            "backup-id": self.snapshot.group().backup_id(),
            "backup-time": self.snapshot.backup_time().timestamp(),
            "files": self.files.iter()
                .fold(Vec::new(), |mut acc, info| {
                    acc.push(json!({
                        "filename": info.filename,
                        "crypt-mode": info.crypt_mode,
                        "size": info.size,
                        "csum": proxmox::tools::digest_to_hex(&info.csum),
                    }));
                    acc
                })
        })
    }

}
impl TryFrom<super::DataBlob> for BackupManifest {
    type Error = Error;

    fn try_from(blob: super::DataBlob) -> Result<Self, Error> {
        let data = blob.decode(None)
            .map_err(|err| format_err!("decode backup manifest blob failed - {}", err))?;
        let json: Value = serde_json::from_slice(&data[..])
            .map_err(|err| format_err!("unable to parse backup manifest json - {}", err))?;
        BackupManifest::try_from(json)
    }
}

impl TryFrom<Value> for BackupManifest {
    type Error = Error;

    fn try_from(data: Value) -> Result<Self, Error> {

        use crate::tools::{required_string_property, required_integer_property, required_array_property};

        proxmox::try_block!({
            let backup_type = required_string_property(&data, "backup-type")?;
            let backup_id = required_string_property(&data, "backup-id")?;
            let backup_time = required_integer_property(&data, "backup-time")?;

            let snapshot = BackupDir::new(backup_type, backup_id, backup_time);

            let mut manifest = BackupManifest::new(snapshot);

            for item in required_array_property(&data, "files")?.iter() {
                let filename = required_string_property(item, "filename")?.to_owned();
                let csum = required_string_property(item, "csum")?;
                let csum = proxmox::tools::hex_to_digest(csum)?;
                let size = required_integer_property(item, "size")? as u64;
                let crypt_mode: CryptMode = serde_json::from_value(item["crypt-mode"].clone())?;
                manifest.add_file(filename, size, csum, crypt_mode)?;
            }

            if manifest.files().is_empty() {
                bail!("manifest does not list any files.");
            }

            Ok(manifest)
        }).map_err(|err: Error| format_err!("unable to parse backup manifest - {}", err))

    }
}
