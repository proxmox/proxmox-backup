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

use crate::api2::types::{
    Userid,
    JOB_ID_SCHEMA,
    DATASTORE_SCHEMA,
    DRIVE_NAME_SCHEMA,
    MEDIA_POOL_NAME_SCHEMA,
    SINGLE_LINE_COMMENT_SCHEMA,
    SYNC_SCHEDULE_SCHEMA,
    JobScheduleStatus,
};

lazy_static! {
    pub static ref CONFIG: SectionConfig = init();
}

#[api(
    properties: {
        store: {
           schema: DATASTORE_SCHEMA,
        },
        pool: {
            schema: MEDIA_POOL_NAME_SCHEMA,
        },
        drive: {
            schema: DRIVE_NAME_SCHEMA,
        },
        "eject-media": {
            description: "Eject media upon job completion.",
            type: bool,
            optional: true,
        },
        "export-media-set": {
            description: "Export media set upon job completion.",
            type: bool,
            optional: true,
        },
        "latest-only": {
            description: "Backup latest snapshots only.",
            type: bool,
            optional: true,
        },
        "notify-user": {
            optional: true,
            type: Userid,
        },
    }
)]
#[derive(Serialize,Deserialize,Clone)]
#[serde(rename_all="kebab-case")]
/// Tape Backup Job Setup
pub struct TapeBackupJobSetup {
    pub store: String,
    pub pool: String,
    pub drive: String,
    #[serde(skip_serializing_if="Option::is_none")]
    pub eject_media: Option<bool>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub export_media_set: Option<bool>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub latest_only: Option<bool>,
    /// Send job email notification to this user
    #[serde(skip_serializing_if="Option::is_none")]
    pub notify_user: Option<Userid>,
}

#[api(
    properties: {
        id: {
            schema: JOB_ID_SCHEMA,
        },
        setup: {
            type: TapeBackupJobSetup,
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
/// Tape Backup Job
pub struct TapeBackupJobConfig {
    pub id: String,
    #[serde(flatten)]
    pub setup: TapeBackupJobSetup,
    #[serde(skip_serializing_if="Option::is_none")]
    pub comment: Option<String>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub schedule: Option<String>,
}

#[api(
    properties: {
        config: {
            type: TapeBackupJobConfig,
        },
        status: {
            type: JobScheduleStatus,
        },
    },
)]
#[derive(Serialize,Deserialize)]
#[serde(rename_all="kebab-case")]
/// Status of Tape Backup Job
pub struct TapeBackupJobStatus {
    #[serde(flatten)]
    pub config: TapeBackupJobConfig,
    #[serde(flatten)]
    pub status: JobScheduleStatus,
}

fn init() -> SectionConfig {
    let obj_schema = match TapeBackupJobConfig::API_SCHEMA {
        Schema::AllOf(ref allof_schema) => allof_schema,
        _ => unreachable!(),
    };

    let plugin = SectionConfigPlugin::new("backup".to_string(), Some(String::from("id")), obj_schema);
    let mut config = SectionConfig::new(&JOB_ID_SCHEMA);
    config.register_plugin(plugin);

    config
}

pub const TAPE_JOB_CFG_FILENAME: &str = "/etc/proxmox-backup/tape-job.cfg";
pub const TAPE_JOB_CFG_LOCKFILE: &str = "/etc/proxmox-backup/.tape-job.lck";

pub fn config() -> Result<(SectionConfigData, [u8;32]), Error> {

    let content = proxmox::tools::fs::file_read_optional_string(TAPE_JOB_CFG_FILENAME)?
        .unwrap_or_else(|| "".to_string());

    let digest = openssl::sha::sha256(content.as_bytes());
    let data = CONFIG.parse(TAPE_JOB_CFG_FILENAME, &content)?;
    Ok((data, digest))
}

pub fn save_config(config: &SectionConfigData) -> Result<(), Error> {
    let raw = CONFIG.write(TAPE_JOB_CFG_FILENAME, &config)?;

    let backup_user = crate::backup::backup_user()?;
    let mode = nix::sys::stat::Mode::from_bits_truncate(0o0640);
    // set the correct owner/group/permissions while saving file
    // owner(rw) = root, group(r)= backup
    let options = CreateOptions::new()
        .perm(mode)
        .owner(nix::unistd::ROOT)
        .group(backup_user.gid);

    replace_file(TAPE_JOB_CFG_FILENAME, raw.as_bytes(), options)?;

    Ok(())
}

// shell completion helper

/// List all tape job IDs
pub fn complete_tape_job_id(_arg: &str, _param: &HashMap<String, String>) -> Vec<String> {
    match config() {
        Ok((data, _digest)) => data.sections.iter().map(|(id, _)| id.to_string()).collect(),
        Err(_) => return vec![],
    }
}
