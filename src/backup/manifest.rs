use failure::*;
use std::convert::TryFrom;

use serde_json::{json, Value};

use crate::backup::BackupDir;

pub const MANIFEST_BLOB_NAME: &str = "index.json.blob";

struct FileInfo {
    filename: String,
    size: u64,
    csum: [u8; 32],
}

pub struct BackupManifest {
    snapshot: BackupDir,
    files: Vec<FileInfo>,
}

impl BackupManifest {

    pub fn new(snapshot: BackupDir) -> Self {
        Self { files: Vec::new(), snapshot }
    }

    pub fn add_file(&mut self, filename: String, size: u64, csum: [u8; 32]) {
        self.files.push(FileInfo { filename, size, csum });
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
                        "size": info.size,
                        "csum": proxmox::tools::digest_to_hex(&info.csum),
                    }));
                    acc
                })
        })
    }

}

impl TryFrom<Value> for BackupManifest {
    type Error = Error;

    fn try_from(data: Value) -> Result<Self, Error> {

        let backup_type = crate::tools::required_string_property(&data, "backup_type")?;
        let backup_id = crate::tools::required_string_property(&data, "backup_id")?;
        let backup_time = crate::tools::required_integer_property(&data, "backup_time")?;

        let snapshot = BackupDir::new(backup_type, backup_id, backup_time);

        let mut files = Vec::new();
        for item in crate::tools::required_array_property(&data, "files")?.iter() {
            let filename = crate::tools::required_string_property(item, "filename")?.to_owned();
            let csum = crate::tools::required_string_property(item, "csum")?;
            let csum = proxmox::tools::hex_to_digest(csum)?;
            let size = crate::tools::required_integer_property(item, "size")? as u64;
            files.push(FileInfo { filename, size, csum });
        }

        Ok(Self { files, snapshot })
    }
}
