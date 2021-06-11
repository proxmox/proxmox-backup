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

use proxmox::tools::fs::{
    open_file_locked,
    replace_file,
    CreateOptions,
};

use crate::api2::types::*;

lazy_static! {
    pub static ref CONFIG: SectionConfig = init();
}


#[api(
    properties: {
        realm: {
            schema: REALM_ID_SCHEMA,
        },
        "issuer-url": {
            description: "OpenID Issuer Url",
            type: String,
        },
        "client-id": {
            description: "OpenID Client ID",
            type: String,
        },
        "client-key": {
            description: "OpenID Client Key",
            type: String,
            optional: true,
        },
        comment: {
            optional: true,
            schema: SINGLE_LINE_COMMENT_SCHEMA,
        },
    },
)]
#[derive(Serialize,Deserialize)]
#[serde(rename_all="kebab-case")]
/// OpenID configuration properties.
pub struct OpenIdRealmConfig {
    pub realm: String,
    pub issuer_url: String,
    pub client_id: String,
    #[serde(skip_serializing_if="Option::is_none")]
    pub client_key: Option<String>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub comment: Option<String>,
}

fn init() -> SectionConfig {
    let obj_schema = match OpenIdRealmConfig::API_SCHEMA {
        Schema::Object(ref obj_schema) => obj_schema,
        _ => unreachable!(),
    };

    let plugin = SectionConfigPlugin::new("openid".to_string(), Some(String::from("realm")), obj_schema);
    let mut config = SectionConfig::new(&REALM_ID_SCHEMA);
    config.register_plugin(plugin);

    config
}

pub const DOMAINS_CFG_FILENAME: &str = "/etc/proxmox-backup/domains.cfg";
pub const DOMAINS_CFG_LOCKFILE: &str = "/etc/proxmox-backup/.domains.lck";

/// Get exclusive lock
pub fn lock_config() -> Result<std::fs::File, Error> {
    open_file_locked(DOMAINS_CFG_LOCKFILE, std::time::Duration::new(10, 0), true)
}

pub fn config() -> Result<(SectionConfigData, [u8;32]), Error> {

    let content = proxmox::tools::fs::file_read_optional_string(DOMAINS_CFG_FILENAME)?
        .unwrap_or_else(|| "".to_string());

    let digest = openssl::sha::sha256(content.as_bytes());
    let data = CONFIG.parse(DOMAINS_CFG_FILENAME, &content)?;
    Ok((data, digest))
}

pub fn save_config(config: &SectionConfigData) -> Result<(), Error> {
    let raw = CONFIG.write(DOMAINS_CFG_FILENAME, &config)?;

    let backup_user = crate::backup::backup_user()?;
    let mode = nix::sys::stat::Mode::from_bits_truncate(0o0640);
    // set the correct owner/group/permissions while saving file
    // owner(rw) = root, group(r)= backup
    let options = CreateOptions::new()
        .perm(mode)
        .owner(nix::unistd::ROOT)
        .group(backup_user.gid);

    replace_file(DOMAINS_CFG_FILENAME, raw.as_bytes(), options)?;

    Ok(())
}

// shell completion helper
pub fn complete_realm_name(_arg: &str, _param: &HashMap<String, String>) -> Vec<String> {
    match config() {
        Ok((data, _digest)) => data.sections.iter().map(|(id, _)| id.to_string()).collect(),
        Err(_) => return vec![],
    }
}

pub fn complete_openid_realm_name(_arg: &str, _param: &HashMap<String, String>) -> Vec<String> {
    match config() {
        Ok((data, _digest)) => data.sections.iter()
            .filter_map(|(id, (t, _))| if t == "openid" { Some(id.to_string()) } else { None })
            .collect(),
        Err(_) => return vec![],
    }
}
