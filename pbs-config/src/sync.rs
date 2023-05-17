use std::collections::HashMap;

use anyhow::Error;
use lazy_static::lazy_static;

use proxmox_schema::{ApiType, Schema};
use proxmox_section_config::{SectionConfig, SectionConfigData, SectionConfigPlugin};

use pbs_api_types::{SyncJobConfig, JOB_ID_SCHEMA};

use crate::{open_backup_lockfile, replace_backup_config, BackupLockGuard};

lazy_static! {
    pub static ref CONFIG: SectionConfig = init();
}

fn init() -> SectionConfig {
    let obj_schema = match SyncJobConfig::API_SCHEMA {
        Schema::AllOf(ref allof_schema) => allof_schema,
        _ => unreachable!(),
    };

    let plugin = SectionConfigPlugin::new("sync".to_string(), Some(String::from("id")), obj_schema);
    let mut config = SectionConfig::new(&JOB_ID_SCHEMA);
    config.register_plugin(plugin);

    config
}

pub const SYNC_CFG_FILENAME: &str = "/etc/proxmox-backup/sync.cfg";
pub const SYNC_CFG_LOCKFILE: &str = "/etc/proxmox-backup/.sync.lck";

/// Get exclusive lock
pub fn lock_config() -> Result<BackupLockGuard, Error> {
    open_backup_lockfile(SYNC_CFG_LOCKFILE, None, true)
}

pub fn config() -> Result<(SectionConfigData, [u8; 32]), Error> {
    let content =
        proxmox_sys::fs::file_read_optional_string(SYNC_CFG_FILENAME)?.unwrap_or_default();

    let digest = openssl::sha::sha256(content.as_bytes());
    let data = CONFIG.parse(SYNC_CFG_FILENAME, &content)?;
    Ok((data, digest))
}

pub fn save_config(config: &SectionConfigData) -> Result<(), Error> {
    let raw = CONFIG.write(SYNC_CFG_FILENAME, config)?;
    replace_backup_config(SYNC_CFG_FILENAME, raw.as_bytes())
}

// shell completion helper
pub fn complete_sync_job_id(_arg: &str, _param: &HashMap<String, String>) -> Vec<String> {
    match config() {
        Ok((data, _digest)) => data.sections.keys().map(|id| id.to_string()).collect(),
        Err(_) => Vec::new(),
    }
}
