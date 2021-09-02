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

use pbs_api_types::{REALM_ID_SCHEMA, SINGLE_LINE_COMMENT_SCHEMA};
use crate::{open_backup_lockfile, replace_backup_config, BackupLockGuard};

lazy_static! {
    pub static ref CONFIG: SectionConfig = init();
}

#[api()]
#[derive(Eq, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
/// Use the value of this attribute/claim as unique user name. It is
/// up to the identity provider to guarantee the uniqueness. The
/// OpenID specification only guarantees that Subject ('sub') is unique. Also
/// make sure that the user is not allowed to change that attribute by
/// himself!
pub enum OpenIdUserAttribute {
    /// Subject (OpenId 'sub' claim)
    Subject,
    /// Username (OpenId 'preferred_username' claim)
    Username,
    /// Email (OpenId 'email' claim)
    Email,
}

#[api(
    properties: {
        realm: {
            schema: REALM_ID_SCHEMA,
        },
        "client-key": {
            optional: true,
        },
        comment: {
            optional: true,
            schema: SINGLE_LINE_COMMENT_SCHEMA,
        },
        autocreate: {
            optional: true,
            default: false,
        },
        "username-claim": {
            type: OpenIdUserAttribute,
            optional: true,
        },
    },
)]
#[derive(Serialize,Deserialize,Updater)]
#[serde(rename_all="kebab-case")]
/// OpenID configuration properties.
pub struct OpenIdRealmConfig {
    #[updater(skip)]
    pub realm: String,
    /// OpenID Issuer Url
    pub issuer_url: String,
    /// OpenID Client ID
    pub client_id: String,
    /// OpenID Client Key
    #[serde(skip_serializing_if="Option::is_none")]
    pub client_key: Option<String>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub comment: Option<String>,
    /// Automatically create users if they do not exist.
    #[serde(skip_serializing_if="Option::is_none")]
    pub autocreate: Option<bool>,
    #[updater(skip)]
    #[serde(skip_serializing_if="Option::is_none")]
    pub username_claim: Option<OpenIdUserAttribute>,
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
pub fn lock_config() -> Result<BackupLockGuard, Error> {
    open_backup_lockfile(DOMAINS_CFG_LOCKFILE, None, true)
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
    replace_backup_config(DOMAINS_CFG_FILENAME, raw.as_bytes())
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
