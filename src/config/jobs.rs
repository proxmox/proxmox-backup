use anyhow::{bail, Error};
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
            schema: GC_SCHEDULE_SCHEMA,
        },
    }
)]
#[serde(rename_all="kebab-case")]
#[derive(Serialize,Deserialize)]
/// Pull Job
pub struct PullJobConfig {
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

fn init() -> SectionConfig {
    let obj_schema = match PullJobConfig::API_SCHEMA {
        Schema::Object(ref obj_schema) => obj_schema,
        _ => unreachable!(),
    };

    let plugin = SectionConfigPlugin::new("pull".to_string(), Some(String::from("id")), obj_schema);
    let mut config = SectionConfig::new(&JOB_ID_SCHEMA);
    config.register_plugin(plugin);

    config
}

pub const JOB_CFG_FILENAME: &str = "/etc/proxmox-backup/job.cfg";
pub const JOB_CFG_LOCKFILE: &str = "/etc/proxmox-backup/.job.lck";

pub fn config() -> Result<(SectionConfigData, [u8;32]), Error> {
    let content = match std::fs::read_to_string(JOB_CFG_FILENAME) {
        Ok(c) => c,
        Err(err) => {
            if err.kind() == std::io::ErrorKind::NotFound {
                String::from("")
            } else {
                bail!("unable to read '{}' - {}", JOB_CFG_FILENAME, err);
            }
        }
    };

    let digest = openssl::sha::sha256(content.as_bytes());
    let data = CONFIG.parse(JOB_CFG_FILENAME, &content)?;
    Ok((data, digest))
}

pub fn save_config(config: &SectionConfigData) -> Result<(), Error> {
    let raw = CONFIG.write(JOB_CFG_FILENAME, &config)?;

    let backup_user = crate::backup::backup_user()?;
    let mode = nix::sys::stat::Mode::from_bits_truncate(0o0640);
    // set the correct owner/group/permissions while saving file
    // owner(rw) = root, group(r)= backup
    let options = CreateOptions::new()
        .perm(mode)
        .owner(nix::unistd::ROOT)
        .group(backup_user.gid);

    replace_file(JOB_CFG_FILENAME, raw.as_bytes(), options)?;

    Ok(())
}

// shell completion helper
pub fn complete_job_id(_arg: &str, _param: &HashMap<String, String>) -> Vec<String> {
    match config() {
        Ok((data, _digest)) => data.sections.iter().map(|(id, _)| id.to_string()).collect(),
        Err(_) => return vec![],
    }
}
