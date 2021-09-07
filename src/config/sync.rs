use anyhow::{Error};
use lazy_static::lazy_static;
use std::collections::HashMap;

use proxmox::api::{
    schema::*,
    section_config::{
        SectionConfig,
        SectionConfigData,
        SectionConfigPlugin,
    }
};

use pbs_api_types::{JOB_ID_SCHEMA, SyncJobConfig};

lazy_static! {
    pub static ref CONFIG: SectionConfig = init();
}


fn init() -> SectionConfig {
    let obj_schema = match SyncJobConfig::API_SCHEMA {
        Schema::Object(ref obj_schema) => obj_schema,
        _ => unreachable!(),
    };

    let plugin = SectionConfigPlugin::new("sync".to_string(), Some(String::from("id")), obj_schema);
    let mut config = SectionConfig::new(&JOB_ID_SCHEMA);
    config.register_plugin(plugin);

    config
}

pub const SYNC_CFG_FILENAME: &str = "/etc/proxmox-backup/sync.cfg";
pub const SYNC_CFG_LOCKFILE: &str = "/etc/proxmox-backup/.sync.lck";

pub fn config() -> Result<(SectionConfigData, [u8;32]), Error> {

    let content = proxmox::tools::fs::file_read_optional_string(SYNC_CFG_FILENAME)?
        .unwrap_or_else(|| "".to_string());

    let digest = openssl::sha::sha256(content.as_bytes());
    let data = CONFIG.parse(SYNC_CFG_FILENAME, &content)?;
    Ok((data, digest))
}

pub fn save_config(config: &SectionConfigData) -> Result<(), Error> {
    let raw = CONFIG.write(SYNC_CFG_FILENAME, &config)?;
    pbs_config::replace_backup_config(SYNC_CFG_FILENAME, raw.as_bytes())
}

// shell completion helper
pub fn complete_sync_job_id(_arg: &str, _param: &HashMap<String, String>) -> Vec<String> {
    match config() {
        Ok((data, _digest)) => data.sections.iter().map(|(id, _)| id.to_string()).collect(),
        Err(_) => return vec![],
    }
}
