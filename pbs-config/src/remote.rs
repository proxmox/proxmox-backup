use std::collections::HashMap;

use anyhow::Error;
use lazy_static::lazy_static;

use proxmox_schema::*;
use proxmox_section_config::{SectionConfig, SectionConfigData, SectionConfigPlugin};

use pbs_api_types::{Remote, REMOTE_ID_SCHEMA};

use crate::{open_backup_lockfile, BackupLockGuard};

lazy_static! {
    pub static ref CONFIG: SectionConfig = init();
}

fn init() -> SectionConfig {
    let obj_schema = match Remote::API_SCHEMA {
        Schema::AllOf(ref allof_schema) => allof_schema,
        _ => unreachable!(),
    };

    let plugin =
        SectionConfigPlugin::new("remote".to_string(), Some("name".to_string()), obj_schema);
    let mut config = SectionConfig::new(&REMOTE_ID_SCHEMA);
    config.register_plugin(plugin);

    config
}

pub const REMOTE_CFG_FILENAME: &str = "/etc/proxmox-backup/remote.cfg";
pub const REMOTE_CFG_LOCKFILE: &str = "/etc/proxmox-backup/.remote.lck";

/// Get exclusive lock
pub fn lock_config() -> Result<BackupLockGuard, Error> {
    open_backup_lockfile(REMOTE_CFG_LOCKFILE, None, true)
}

pub fn config() -> Result<(SectionConfigData, [u8; 32]), Error> {
    let content =
        proxmox_sys::fs::file_read_optional_string(REMOTE_CFG_FILENAME)?.unwrap_or_default();

    let digest = openssl::sha::sha256(content.as_bytes());
    let data = CONFIG.parse(REMOTE_CFG_FILENAME, &content)?;
    Ok((data, digest))
}

pub fn save_config(config: &SectionConfigData) -> Result<(), Error> {
    let raw = CONFIG.write(REMOTE_CFG_FILENAME, config)?;
    crate::replace_backup_config(REMOTE_CFG_FILENAME, raw.as_bytes())
}

// shell completion helper
pub fn complete_remote_name(_arg: &str, _param: &HashMap<String, String>) -> Vec<String> {
    match config() {
        Ok((data, _digest)) => data.sections.keys().map(|id| id.to_string()).collect(),
        Err(_) => Vec::new(),
    }
}
