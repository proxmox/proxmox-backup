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
        "ignore-verified": {
            optional: true,
            schema: IGNORE_VERIFIED_BACKUPS_SCHEMA,
        },
        "outdated-after": {
            optional: true,
            schema: VERIFICATION_OUTDATED_AFTER_SCHEMA,
        },
        comment: {
            optional: true,
            schema: SINGLE_LINE_COMMENT_SCHEMA,
        },
        schedule: {
            optional: true,
            schema: VERIFICATION_SCHEDULE_SCHEMA,
        },
    }
)]
#[serde(rename_all="kebab-case")]
#[derive(Serialize,Deserialize)]
/// Verification Job
pub struct VerificationJobConfig {
    /// unique ID to address this job
    pub id: String,
    /// the datastore ID this verificaiton job affects
    pub store: String,
    #[serde(skip_serializing_if="Option::is_none")]
    /// if not set to false, check the age of the last snapshot verification to filter
    /// out recent ones, depending on 'outdated_after' configuration.
    pub ignore_verified: Option<bool>,
    #[serde(skip_serializing_if="Option::is_none")]
    /// Reverify snapshots after X days, never if 0. Ignored if 'ignore_verified' is false.
    pub outdated_after: Option<i64>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub comment: Option<String>,
    #[serde(skip_serializing_if="Option::is_none")]
    /// when to schedule this job in calendar event notation
    pub schedule: Option<String>,
}


#[api(
    properties: {
        id: {
            schema: JOB_ID_SCHEMA,
        },
        store: {
            schema: DATASTORE_SCHEMA,
        },
        "ignore-verified": {
            optional: true,
            schema: IGNORE_VERIFIED_BACKUPS_SCHEMA,
        },
        "outdated-after": {
            optional: true,
            schema: VERIFICATION_OUTDATED_AFTER_SCHEMA,
        },
        comment: {
            optional: true,
            schema: SINGLE_LINE_COMMENT_SCHEMA,
        },
        schedule: {
            optional: true,
            schema: VERIFICATION_SCHEDULE_SCHEMA,
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
/// Status of Verification Job
pub struct VerificationJobStatus {
    /// unique ID to address this job
    pub id: String,
    /// the datastore ID this verificaiton job affects
    pub store: String,
    #[serde(skip_serializing_if="Option::is_none")]
    /// if not set to false, check the age of the last snapshot verification to filter
    /// out recent ones, depending on 'outdated_after' configuration.
    pub ignore_verified: Option<bool>,
    #[serde(skip_serializing_if="Option::is_none")]
    /// Reverify snapshots after X days, never if 0. Ignored if 'ignore_verified' is false.
    pub outdated_after: Option<i64>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub comment: Option<String>,
    #[serde(skip_serializing_if="Option::is_none")]
    /// when to schedule this job in calendar event notation
    pub schedule: Option<String>,
    #[serde(skip_serializing_if="Option::is_none")]
    /// The timestamp when this job runs the next time.
    pub next_run: Option<i64>,
    #[serde(skip_serializing_if="Option::is_none")]
    /// The state of the last scheduled run, if any
    pub last_run_state: Option<String>,
    #[serde(skip_serializing_if="Option::is_none")]
    /// The task UPID of the last scheduled run, if any
    pub last_run_upid: Option<String>,
    #[serde(skip_serializing_if="Option::is_none")]
    /// When the last run was finished, combined with UPID.starttime one can calculate the duration
    pub last_run_endtime: Option<i64>,
}


fn init() -> SectionConfig {
    let obj_schema = match VerificationJobConfig::API_SCHEMA {
        Schema::Object(ref obj_schema) => obj_schema,
        _ => unreachable!(),
    };

    let plugin = SectionConfigPlugin::new("verification".to_string(), Some(String::from("id")), obj_schema);
    let mut config = SectionConfig::new(&JOB_ID_SCHEMA);
    config.register_plugin(plugin);

    config
}

pub const VERIFICATION_CFG_FILENAME: &str = "/etc/proxmox-backup/verification.cfg";
pub const VERIFICATION_CFG_LOCKFILE: &str = "/etc/proxmox-backup/.verification.lck";

pub fn config() -> Result<(SectionConfigData, [u8;32]), Error> {

    let content = proxmox::tools::fs::file_read_optional_string(VERIFICATION_CFG_FILENAME)?;
    let content = content.unwrap_or_else(String::new);

    let digest = openssl::sha::sha256(content.as_bytes());
    let data = CONFIG.parse(VERIFICATION_CFG_FILENAME, &content)?;
    Ok((data, digest))
}

pub fn save_config(config: &SectionConfigData) -> Result<(), Error> {
    let raw = CONFIG.write(VERIFICATION_CFG_FILENAME, &config)?;

    let backup_user = crate::backup::backup_user()?;
    let mode = nix::sys::stat::Mode::from_bits_truncate(0o0640);
    // set the correct owner/group/permissions while saving file
    // owner(rw) = root, group(r)= backup

    let options = CreateOptions::new()
        .perm(mode)
        .owner(nix::unistd::ROOT)
        .group(backup_user.gid);

    replace_file(VERIFICATION_CFG_FILENAME, raw.as_bytes(), options)?;

    Ok(())
}

// shell completion helper
pub fn complete_verification_job_id(_arg: &str, _param: &HashMap<String, String>) -> Vec<String> {
    match config() {
        Ok((data, _digest)) => data.sections.iter().map(|(id, _)| id.to_string()).collect(),
        Err(_) => return vec![],
    }
}
