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

use proxmox::tools::{fs::replace_file, fs::CreateOptions};

use crate::api2::types::*;

lazy_static! {
    static ref CONFIG: SectionConfig = init();
}


#[api(
    properties: {
        id: {
            schema: JOB_ID_SCHEMA,
        },
        store: {
           schema: DATASTORE_SCHEMA,
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
#[serde(rename_all="kebab-case")]
#[derive(Serialize,Deserialize,Clone)]
/// Sync Job
pub struct SyncJobConfig {
    pub id: String,
    pub store: String,
    pub remote: String,
    pub remote_store: String,
    #[serde(skip_serializing_if="Option::is_none")]
    pub remove_vanished: Option<bool>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub comment: Option<String>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub schedule: Option<String>,
}

// FIXME: generate duplicate schemas/structs from one listing?
#[api(
    properties: {
        id: {
            schema: JOB_ID_SCHEMA,
        },
        store: {
           schema: DATASTORE_SCHEMA,
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
        "next-run": {
            description: "Estimated time of the next run (UNIX epoch).",
            optional: true,
            type: Integer,
        },
        "last-run-state": {
            description: "Result of the last run.",
            optional: true,
            type: String,
        },
        "last-run-upid": {
            description: "Task UPID of the last run.",
            optional: true,
            type: String,
        },
        "last-run-endtime": {
            description: "Endtime of the last run.",
            optional: true,
            type: Integer,
        },
    }
)]
#[serde(rename_all="kebab-case")]
#[derive(Serialize,Deserialize)]
/// Status of Sync Job
pub struct SyncJobStatus {
    pub id: String,
    pub store: String,
    pub remote: String,
    pub remote_store: String,
    #[serde(skip_serializing_if="Option::is_none")]
    pub remove_vanished: Option<bool>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub comment: Option<String>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub schedule: Option<String>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub next_run: Option<i64>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub last_run_state: Option<String>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub last_run_upid: Option<String>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub last_run_endtime: Option<i64>,
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

    let content = proxmox::tools::fs::file_read_optional_string(SYNC_CFG_FILENAME)?;
    let content = content.unwrap_or(String::from(""));

    let digest = openssl::sha::sha256(content.as_bytes());
    let data = CONFIG.parse(SYNC_CFG_FILENAME, &content)?;
    Ok((data, digest))
}

pub fn save_config(config: &SectionConfigData) -> Result<(), Error> {
    let raw = CONFIG.write(SYNC_CFG_FILENAME, &config)?;

    let backup_user = crate::backup::backup_user()?;
    let mode = nix::sys::stat::Mode::from_bits_truncate(0o0640);
    // set the correct owner/group/permissions while saving file
    // owner(rw) = root, group(r)= backup
    let options = CreateOptions::new()
        .perm(mode)
        .owner(nix::unistd::ROOT)
        .group(backup_user.gid);

    replace_file(SYNC_CFG_FILENAME, raw.as_bytes(), options)?;

    Ok(())
}

// shell completion helper
pub fn complete_sync_job_id(_arg: &str, _param: &HashMap<String, String>) -> Vec<String> {
    match config() {
        Ok((data, _digest)) => data.sections.iter().map(|(id, _)| id.to_string()).collect(),
        Err(_) => return vec![],
    }
}
