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

        let backup_type = data["backup_type"].as_str().unwrap();
        let backup_id = data["backup_id"].as_str().unwrap();
        let backup_time = data["backup_time"].as_i64().unwrap();

        let snapshot = BackupDir::new(backup_type, backup_id, backup_time);

        let files = Vec::new();

        Ok(Self { files, snapshot })
    }
}
