//! Traffic Control Settings (Network rate limits)
use std::collections::HashMap;

use anyhow::Error;
use lazy_static::lazy_static;

use proxmox_schema::{ApiType, Schema};

use pbs_api_types::{TrafficControlRule, TRAFFIC_CONTROL_ID_SCHEMA};

use proxmox_section_config::{SectionConfig, SectionConfigData, SectionConfigPlugin};

use crate::ConfigVersionCache;
use crate::{open_backup_lockfile, replace_backup_config, BackupLockGuard};

lazy_static! {
    /// Static [`SectionConfig`] to access parser/writer functions.
    pub static ref CONFIG: SectionConfig = init();
}

fn init() -> SectionConfig {
    let mut config = SectionConfig::new(&TRAFFIC_CONTROL_ID_SCHEMA);

    let obj_schema = match TrafficControlRule::API_SCHEMA {
        Schema::AllOf(ref allof_schema) => allof_schema,
        _ => unreachable!(),
    };
    let plugin = SectionConfigPlugin::new("rule".to_string(), Some("name".to_string()), obj_schema);
    config.register_plugin(plugin);

    config
}

/// Configuration file name
pub const TRAFFIC_CONTROL_CFG_FILENAME: &str = "/etc/proxmox-backup/traffic-control.cfg";
/// Lock file name (used to prevent concurrent access)
pub const TRAFFIC_CONTROL_CFG_LOCKFILE: &str = "/etc/proxmox-backup/.traffic-control.lck";

/// Get exclusive lock
pub fn lock_config() -> Result<BackupLockGuard, Error> {
    open_backup_lockfile(TRAFFIC_CONTROL_CFG_LOCKFILE, None, true)
}

/// Read and parse the configuration file
pub fn config() -> Result<(SectionConfigData, [u8; 32]), Error> {
    let content = proxmox_sys::fs::file_read_optional_string(TRAFFIC_CONTROL_CFG_FILENAME)?
        .unwrap_or_default();

    let digest = openssl::sha::sha256(content.as_bytes());
    let data = CONFIG.parse(TRAFFIC_CONTROL_CFG_FILENAME, &content)?;
    Ok((data, digest))
}

/// Save the configuration file
pub fn save_config(config: &SectionConfigData) -> Result<(), Error> {
    let raw = CONFIG.write(TRAFFIC_CONTROL_CFG_FILENAME, config)?;
    replace_backup_config(TRAFFIC_CONTROL_CFG_FILENAME, raw.as_bytes())?;

    // increase traffic control version
    // We use this in TrafficControlCache
    let version_cache = ConfigVersionCache::new()?;
    version_cache.increase_traffic_control_generation();

    Ok(())
}

// shell completion helper
pub fn complete_traffic_control_name(_arg: &str, _param: &HashMap<String, String>) -> Vec<String> {
    match config() {
        Ok((data, _digest)) => data.sections.keys().map(|id| id.to_string()).collect(),
        Err(_) => Vec::new(),
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test1() -> Result<(), Error> {
        let content = "rule: rule1
 comment localnet at working hours
 network 192.168.2.0/24
 network 192.168.3.0/24
 rate-in 500000
 timeframe mon..wed 8:00-16:30
 timeframe fri 9:00-12:00
";
        let data = CONFIG.parse(TRAFFIC_CONTROL_CFG_FILENAME, content)?;
        eprintln!("GOT {:?}", data);

        Ok(())
    }
}
