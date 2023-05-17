use anyhow::Error;
use lazy_static::lazy_static;
use std::collections::HashMap;

use proxmox_schema::{AllOfSchema, ApiType};
use proxmox_section_config::{SectionConfig, SectionConfigData, SectionConfigPlugin};

use pbs_api_types::{DataStoreConfig, DATASTORE_SCHEMA};

use crate::{open_backup_lockfile, replace_backup_config, BackupLockGuard, ConfigVersionCache};

lazy_static! {
    pub static ref CONFIG: SectionConfig = init();
}

fn init() -> SectionConfig {
    const OBJ_SCHEMA: &AllOfSchema = DataStoreConfig::API_SCHEMA.unwrap_all_of_schema();

    let plugin = SectionConfigPlugin::new(
        "datastore".to_string(),
        Some(String::from("name")),
        OBJ_SCHEMA,
    );
    let mut config = SectionConfig::new(&DATASTORE_SCHEMA);
    config.register_plugin(plugin);

    config
}

pub const DATASTORE_CFG_FILENAME: &str = "/etc/proxmox-backup/datastore.cfg";
pub const DATASTORE_CFG_LOCKFILE: &str = "/etc/proxmox-backup/.datastore.lck";

/// Get exclusive lock
pub fn lock_config() -> Result<BackupLockGuard, Error> {
    open_backup_lockfile(DATASTORE_CFG_LOCKFILE, None, true)
}

pub fn config() -> Result<(SectionConfigData, [u8; 32]), Error> {
    let content =
        proxmox_sys::fs::file_read_optional_string(DATASTORE_CFG_FILENAME)?.unwrap_or_default();

    let digest = openssl::sha::sha256(content.as_bytes());
    let data = CONFIG.parse(DATASTORE_CFG_FILENAME, &content)?;
    Ok((data, digest))
}

pub fn save_config(config: &SectionConfigData) -> Result<(), Error> {
    let raw = CONFIG.write(DATASTORE_CFG_FILENAME, config)?;
    replace_backup_config(DATASTORE_CFG_FILENAME, raw.as_bytes())?;

    // used in pbs-datastore
    let version_cache = ConfigVersionCache::new()?;
    version_cache.increase_datastore_generation();

    Ok(())
}

// shell completion helper
pub fn complete_datastore_name(_arg: &str, _param: &HashMap<String, String>) -> Vec<String> {
    match config() {
        Ok((data, _digest)) => data.sections.keys().map(|id| id.to_string()).collect(),
        Err(_) => Vec::new(),
    }
}

pub fn complete_acl_path(_arg: &str, _param: &HashMap<String, String>) -> Vec<String> {
    let mut list = vec![
        String::from("/"),
        String::from("/datastore"),
        String::from("/datastore/"),
    ];

    if let Ok((data, _digest)) = config() {
        for id in data.sections.keys() {
            list.push(format!("/datastore/{}", id));
        }
    }

    list.push(String::from("/remote"));
    list.push(String::from("/remote/"));

    list.push(String::from("/tape"));
    list.push(String::from("/tape/"));
    list.push(String::from("/tape/drive"));
    list.push(String::from("/tape/drive/"));
    list.push(String::from("/tape/changer"));
    list.push(String::from("/tape/changer/"));
    list.push(String::from("/tape/pool"));
    list.push(String::from("/tape/pool/"));
    list.push(String::from("/tape/job"));
    list.push(String::from("/tape/job/"));

    list
}

pub fn complete_calendar_event(_arg: &str, _param: &HashMap<String, String>) -> Vec<String> {
    // just give some hints about possible values
    ["minutely", "hourly", "daily", "mon..fri", "0:0"]
        .iter()
        .map(|s| String::from(*s))
        .collect()
}
