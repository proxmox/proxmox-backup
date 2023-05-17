use std::collections::HashMap;

use anyhow::Error;
use lazy_static::lazy_static;

use pbs_buildcfg::configdir;
use proxmox_schema::{ApiType, ObjectSchema};
use proxmox_section_config::{SectionConfig, SectionConfigData, SectionConfigPlugin};

use crate::{open_backup_lockfile, replace_backup_config, BackupLockGuard};
use pbs_api_types::{LdapRealmConfig, OpenIdRealmConfig, REALM_ID_SCHEMA};

lazy_static! {
    pub static ref CONFIG: SectionConfig = init();
}

fn init() -> SectionConfig {
    const LDAP_SCHEMA: &ObjectSchema = LdapRealmConfig::API_SCHEMA.unwrap_object_schema();
    const OPENID_SCHEMA: &ObjectSchema = OpenIdRealmConfig::API_SCHEMA.unwrap_object_schema();

    let mut config = SectionConfig::new(&REALM_ID_SCHEMA);

    let plugin = SectionConfigPlugin::new(
        "openid".to_string(),
        Some(String::from("realm")),
        OPENID_SCHEMA,
    );

    config.register_plugin(plugin);

    let plugin =
        SectionConfigPlugin::new("ldap".to_string(), Some(String::from("realm")), LDAP_SCHEMA);

    config.register_plugin(plugin);

    config
}

pub const DOMAINS_CFG_FILENAME: &str = configdir!("/domains.cfg");
pub const DOMAINS_CFG_LOCKFILE: &str = configdir!("/.domains.lck");

/// Get exclusive lock
pub fn lock_config() -> Result<BackupLockGuard, Error> {
    open_backup_lockfile(DOMAINS_CFG_LOCKFILE, None, true)
}

pub fn config() -> Result<(SectionConfigData, [u8; 32]), Error> {
    let content =
        proxmox_sys::fs::file_read_optional_string(DOMAINS_CFG_FILENAME)?.unwrap_or_default();

    let digest = openssl::sha::sha256(content.as_bytes());
    let data = CONFIG.parse(DOMAINS_CFG_FILENAME, &content)?;
    Ok((data, digest))
}

pub fn save_config(config: &SectionConfigData) -> Result<(), Error> {
    let raw = CONFIG.write(DOMAINS_CFG_FILENAME, config)?;
    replace_backup_config(DOMAINS_CFG_FILENAME, raw.as_bytes())
}

/// Check if a realm with the given name exists
pub fn exists(domains: &SectionConfigData, realm: &str) -> bool {
    realm == "pbs" || realm == "pam" || domains.sections.get(realm).is_some()
}

// shell completion helper
pub fn complete_realm_name(_arg: &str, _param: &HashMap<String, String>) -> Vec<String> {
    match config() {
        Ok((data, _digest)) => data.sections.keys().map(|id| id.to_string()).collect(),
        Err(_) => Vec::new(),
    }
}

fn complete_realm_of_type(realm_type: &str) -> Vec<String> {
    match config() {
        Ok((data, _digest)) => data
            .sections
            .iter()
            .filter_map(|(id, (t, _))| {
                if t == realm_type {
                    Some(id.to_string())
                } else {
                    None
                }
            })
            .collect(),
        Err(_) => Vec::new(),
    }
}

pub fn complete_openid_realm_name(_arg: &str, _param: &HashMap<String, String>) -> Vec<String> {
    complete_realm_of_type("openid")
}

pub fn complete_ldap_realm_name(_arg: &str, _param: &HashMap<String, String>) -> Vec<String> {
    complete_realm_of_type("ldap")
}
