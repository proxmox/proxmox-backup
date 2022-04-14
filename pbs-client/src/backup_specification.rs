use anyhow::{bail, Error};

use proxmox_schema::*;

const_regex! {
    BACKUPSPEC_REGEX = r"^([a-zA-Z0-9_-]+\.(pxar|img|conf|log)):(.+)$";
}

pub const BACKUP_SOURCE_SCHEMA: Schema =
    StringSchema::new("Backup source specification ([<label>:<path>]).")
        .format(&ApiStringFormat::Pattern(&BACKUPSPEC_REGEX))
        .schema();

pub enum BackupSpecificationType {
    PXAR,
    IMAGE,
    CONFIG,
    LOGFILE,
}

pub struct BackupSpecification {
    pub archive_name: String,  // left part
    pub config_string: String, // right part
    pub spec_type: BackupSpecificationType,
}

pub fn parse_backup_specification(value: &str) -> Result<BackupSpecification, Error> {
    if let Some(caps) = (BACKUPSPEC_REGEX.regex_obj)().captures(value) {
        let archive_name = caps.get(1).unwrap().as_str().into();
        let extension = caps.get(2).unwrap().as_str();
        let config_string = caps.get(3).unwrap().as_str().into();
        let spec_type = match extension {
            "pxar" => BackupSpecificationType::PXAR,
            "img" => BackupSpecificationType::IMAGE,
            "conf" => BackupSpecificationType::CONFIG,
            "log" => BackupSpecificationType::LOGFILE,
            _ => bail!("unknown backup source type '{}'", extension),
        };
        return Ok(BackupSpecification {
            archive_name,
            config_string,
            spec_type,
        });
    }

    bail!("unable to parse backup source specification '{}'", value);
}
