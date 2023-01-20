use std::collections::HashMap;

use anyhow::Error;
use lazy_static::lazy_static;

use proxmox_schema::*;
use proxmox_section_config::{SectionConfig, SectionConfigData, SectionConfigPlugin};

use pbs_api_types::{InfluxDbHttp, InfluxDbUdp, METRIC_SERVER_ID_SCHEMA};

use crate::{open_backup_lockfile, BackupLockGuard};

lazy_static! {
    pub static ref CONFIG: SectionConfig = init();
}

fn init() -> SectionConfig {
    let mut config = SectionConfig::new(&METRIC_SERVER_ID_SCHEMA);

    const UDP_SCHEMA: &ObjectSchema = InfluxDbUdp::API_SCHEMA.unwrap_object_schema();
    let udp_plugin = SectionConfigPlugin::new(
        "influxdb-udp".to_string(),
        Some("name".to_string()),
        UDP_SCHEMA,
    );
    config.register_plugin(udp_plugin);

    const HTTP_SCHEMA: &ObjectSchema = InfluxDbHttp::API_SCHEMA.unwrap_object_schema();

    let http_plugin = SectionConfigPlugin::new(
        "influxdb-http".to_string(),
        Some("name".to_string()),
        HTTP_SCHEMA,
    );

    config.register_plugin(http_plugin);

    config
}

pub const METRIC_SERVER_CFG_FILENAME: &str = "/etc/proxmox-backup/metricserver.cfg";
pub const METRIC_SERVER_CFG_LOCKFILE: &str = "/etc/proxmox-backup/.metricserver.lck";

/// Get exclusive lock
pub fn lock_config() -> Result<BackupLockGuard, Error> {
    open_backup_lockfile(METRIC_SERVER_CFG_LOCKFILE, None, true)
}

pub fn config() -> Result<(SectionConfigData, [u8; 32]), Error> {
    let content =
        proxmox_sys::fs::file_read_optional_string(METRIC_SERVER_CFG_FILENAME)?.unwrap_or_default();

    let digest = openssl::sha::sha256(content.as_bytes());
    let data = CONFIG.parse(METRIC_SERVER_CFG_FILENAME, &content)?;
    Ok((data, digest))
}

pub fn save_config(config: &SectionConfigData) -> Result<(), Error> {
    let raw = CONFIG.write(METRIC_SERVER_CFG_FILENAME, config)?;
    crate::replace_backup_config(METRIC_SERVER_CFG_FILENAME, raw.as_bytes())
}

// shell completion helper
pub fn complete_remote_name(_arg: &str, _param: &HashMap<String, String>) -> Vec<String> {
    match config() {
        Ok((data, _digest)) => data.sections.keys().cloned().collect(),
        Err(_) => Vec::new(),
    }
}
