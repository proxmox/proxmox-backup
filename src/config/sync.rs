use anyhow::{Error};
use lazy_static::lazy_static;
use std::collections::HashMap;
use serde::{Serialize, Deserialize};

use proxmox::api::{
    api,
    schema::*,
    section_config::{
        SectionConfig,
        SectionConfigData,
        SectionConfigPlugin,
    }
};

use crate::api2::types::*;

lazy_static! {
    pub static ref CONFIG: SectionConfig = init();
}

#[api(
    properties: {
        id: {
            schema: JOB_ID_SCHEMA,
        },
        store: {
           schema: DATASTORE_SCHEMA,
        },
        "owner": {
            type: Authid,
            optional: true,
        },
        remote: {
            schema: REMOTE_ID_SCHEMA,
        },
        "remote-store": {
            schema: DATASTORE_SCHEMA,
        },
        "remove-vanished": {
            schema: REMOVE_VANISHED_BACKUPS_SCHEMA,
            optional: true,
        },
        comment: {
            optional: true,
            schema: SINGLE_LINE_COMMENT_SCHEMA,
        },
        schedule: {
            optional: true,
            schema: SYNC_SCHEDULE_SCHEMA,
        },
    }
)]
#[derive(Serialize,Deserialize,Clone)]
#[serde(rename_all="kebab-case")]
/// Sync Job
pub struct SyncJobConfig {
    pub id: String,
    pub store: String,
    #[serde(skip_serializing_if="Option::is_none")]
    pub owner: Option<Authid>,
    pub remote: String,
    pub remote_store: String,
    #[serde(skip_serializing_if="Option::is_none")]
    pub remove_vanished: Option<bool>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub comment: Option<String>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub schedule: Option<String>,
}

#[api(
    properties: {
        config: {
            type: SyncJobConfig,
        },
        status: {
            type: JobScheduleStatus,
        },
    },
)]

#[derive(Serialize,Deserialize)]
#[serde(rename_all="kebab-case")]
/// Status of Sync Job
pub struct SyncJobStatus {
    #[serde(flatten)]
    pub config: SyncJobConfig,
    #[serde(flatten)]
    pub status: JobScheduleStatus,
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
