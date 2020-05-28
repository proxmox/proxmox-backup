use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use anyhow::{bail, Error};
use lazy_static::lazy_static;
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

pub const ENABLE_USER_SCHEMA: Schema = BooleanSchema::new(
    "Enable the account (default). You can set this to '0' to disable the account.")
    .default(true)
    .schema();

pub const EXPIRE_USER_SCHEMA: Schema = IntegerSchema::new(
    "Account expiration date (seconds since epoch). '0' means no expiration date.")
    .default(0)
    .minimum(0)
    .schema();

pub const FIRST_NAME_SCHEMA: Schema = StringSchema::new("First name.")
    .format(&SINGLE_LINE_COMMENT_FORMAT)
    .min_length(2)
    .max_length(64)
    .schema();

pub const LAST_NAME_SCHEMA: Schema = StringSchema::new("Last name.")
    .format(&SINGLE_LINE_COMMENT_FORMAT)
    .min_length(2)
    .max_length(64)
    .schema();

pub const EMAIL_SCHEMA: Schema = StringSchema::new("E-Mail Address.")
    .format(&SINGLE_LINE_COMMENT_FORMAT)
    .min_length(2)
    .max_length(64)
    .schema();


#[api(
    properties: {
        userid: {
            schema: PROXMOX_USER_ID_SCHEMA,
        },
        comment: {
            optional: true,
            schema: SINGLE_LINE_COMMENT_SCHEMA,
        },
        enable: {
            optional: true,
            schema: ENABLE_USER_SCHEMA,
        },
        expire: {
            optional: true,
            schema: EXPIRE_USER_SCHEMA,
        },
        firstname: {
            optional: true,
            schema: FIRST_NAME_SCHEMA,
        },
        lastname: {
            schema: LAST_NAME_SCHEMA,
            optional: true,
         },
        email: {
            schema: EMAIL_SCHEMA,
            optional: true,
        },
    }
)]
#[derive(Serialize,Deserialize)]
/// User properties.
pub struct User {
    pub userid: String,
    #[serde(skip_serializing_if="Option::is_none")]
    pub comment: Option<String>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub enable: Option<bool>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub expire: Option<i64>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub firstname: Option<String>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub lastname: Option<String>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub email: Option<String>,
}

fn init() -> SectionConfig {
    let obj_schema = match User::API_SCHEMA {
        Schema::Object(ref obj_schema) => obj_schema,
        _ => unreachable!(),
    };

    let plugin = SectionConfigPlugin::new("user".to_string(), Some("userid".to_string()), obj_schema);
    let mut config = SectionConfig::new(&PROXMOX_USER_ID_SCHEMA);

    config.register_plugin(plugin);

    config
}

pub const USER_CFG_FILENAME: &str = "/etc/proxmox-backup/user.cfg";
pub const USER_CFG_LOCKFILE: &str = "/etc/proxmox-backup/.user.lck";

pub fn config() -> Result<(SectionConfigData, [u8;32]), Error> {

    let content = proxmox::tools::fs::file_read_optional_string(USER_CFG_FILENAME)?;
    let content = content.unwrap_or(String::from(""));

    let digest = openssl::sha::sha256(content.as_bytes());
    let mut data = CONFIG.parse(USER_CFG_FILENAME, &content)?;

    if data.sections.get("root@pam").is_none() {
        let user: User = User {
            userid: "root@pam".to_string(),
            comment: Some("Superuser".to_string()),
            enable: None,
            expire: None,
            firstname: None,
            lastname: None,
            email: None,
        };
        data.set_data("root@pam", "user", &user).unwrap();
    }

    Ok((data, digest))
}

pub fn cached_config() -> Result<Arc<SectionConfigData>, Error> {

    struct ConfigCache {
        data: Option<Arc<SectionConfigData>>,
        last_mtime: i64,
        last_mtime_nsec: i64,
    }

    lazy_static! {
        static ref CACHED_CONFIG: RwLock<ConfigCache> = RwLock::new(
            ConfigCache { data: None, last_mtime: 0, last_mtime_nsec: 0 });
    }

    let stat = match nix::sys::stat::stat(USER_CFG_FILENAME) {
        Ok(stat) => Some(stat),
        Err(nix::Error::Sys(nix::errno::Errno::ENOENT)) => None,
        Err(err) => bail!("unable to stat '{}' - {}", USER_CFG_FILENAME, err),
    };

    { // limit scope
        let cache = CACHED_CONFIG.read().unwrap();
        if let Some(ref config) = cache.data {
            if let Some(stat) = stat {
                if stat.st_mtime == cache.last_mtime && stat.st_mtime_nsec == cache.last_mtime_nsec {
                    return Ok(config.clone());
                }
            } else if cache.last_mtime == 0 && cache.last_mtime_nsec == 0 {
                return Ok(config.clone());
            }
        }
    }

    let (config, _digest) = config()?;
    let config = Arc::new(config);

    let mut cache = CACHED_CONFIG.write().unwrap();
    if let Some(stat) = stat {
        cache.last_mtime = stat.st_mtime;
        cache.last_mtime_nsec = stat.st_mtime_nsec;
    }
    cache.data = Some(config.clone());

    Ok(config)
}

pub fn save_config(config: &SectionConfigData) -> Result<(), Error> {
    let raw = CONFIG.write(USER_CFG_FILENAME, &config)?;

    let backup_user = crate::backup::backup_user()?;
    let mode = nix::sys::stat::Mode::from_bits_truncate(0o0640);
    // set the correct owner/group/permissions while saving file
    // owner(rw) = root, group(r)= backup
    let options = CreateOptions::new()
        .perm(mode)
        .owner(nix::unistd::ROOT)
        .group(backup_user.gid);

    replace_file(USER_CFG_FILENAME, raw.as_bytes(), options)?;

    Ok(())
}

// shell completion helper
pub fn complete_user_name(_arg: &str, _param: &HashMap<String, String>) -> Vec<String> {
    match config() {
        Ok((data, _digest)) => data.sections.iter().map(|(id, _)| id.to_string()).collect(),
        Err(_) => return vec![],
    }
}
