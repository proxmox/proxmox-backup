use failure::*;
use lazy_static::lazy_static;
use std::collections::HashMap;
use serde::{Serialize, Deserialize};

use proxmox::api::{api, schema::*};

use proxmox::tools::{fs::replace_file, fs::CreateOptions};

use crate::api2::types::*;
use crate::section_config::{SectionConfig, SectionConfigData, SectionConfigPlugin};

lazy_static! {
    static ref CONFIG: SectionConfig = init();
}

// fixme: define better schemas

pub const COMMENT_SCHEMA: Schema = StringSchema::new("Comment").schema();
pub const REMOTE_HOST_SCHEMA: Schema = StringSchema::new("Host IP address or DNS name.").schema();
pub const REMOTE_USERID_SCHEMA: Schema = StringSchema::new("User ID").schema();
pub const REMOTE_PASSWORD_SCHEMA: Schema = StringSchema::new("Password or auth token.").schema();

#[api(
    properties: {
        comment: {
            optional: true,
            schema: COMMENT_SCHEMA,
        },
        host: {
            schema: REMOTE_HOST_SCHEMA,
        },
        userid: {
            schema: REMOTE_USERID_SCHEMA,
        },
        password: {
            schema: REMOTE_PASSWORD_SCHEMA,
        },
    }
)]
#[derive(Serialize,Deserialize)]
/// Remote properties.
pub struct Remote {
    pub comment: Option<String>,
    pub host: String,
    pub userid: String,
    pub password: String,
}

fn init() -> SectionConfig {
    let obj_schema = match Remote::API_SCHEMA {
        Schema::Object(ref obj_schema) => obj_schema,
        _ => unreachable!(),
    };

    let plugin = SectionConfigPlugin::new("remote".to_string(), obj_schema);
    let mut config = SectionConfig::new(&REMOTE_ID_SCHEMA);
    config.register_plugin(plugin);

    config
}

const REMOTES_CFG_FILENAME: &str = "/etc/proxmox-backup/remotes.cfg";

pub fn config() -> Result<SectionConfigData, Error> {
    let content = match std::fs::read_to_string(REMOTES_CFG_FILENAME) {
        Ok(c) => c,
        Err(err) => {
            if err.kind() == std::io::ErrorKind::NotFound {
                String::from("")
            } else {
                bail!("unable to read '{}' - {}", REMOTES_CFG_FILENAME, err);
            }
        }
    };

    CONFIG.parse(REMOTES_CFG_FILENAME, &content)
}

pub fn save_config(config: &SectionConfigData) -> Result<(), Error> {
    let raw = CONFIG.write(REMOTES_CFG_FILENAME, &config)?;

    let backup_user = crate::backup::backup_user()?;
    let mode = nix::sys::stat::Mode::from_bits_truncate(0o0640);
    // set the correct owner/group/permissions while saving file
    // owner(rw) = root, group(r)= backup
    let options = CreateOptions::new()
        .perm(mode)
        .owner(nix::unistd::ROOT)
        .group(backup_user.gid);

    replace_file(REMOTES_CFG_FILENAME, raw.as_bytes(), options)?;

    Ok(())
}

// shell completion helper
pub fn complete_remote_name(_arg: &str, _param: &HashMap<String, String>) -> Vec<String> {
    match config() {
        Ok(data) => data.sections.iter().map(|(id, _)| id.to_string()).collect(),
        Err(_) => return vec![],
    }
}
