use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use anyhow::{bail, Error};
use lazy_static::lazy_static;

use proxmox_schema::*;
use proxmox_section_config::{SectionConfig, SectionConfigData, SectionConfigPlugin};

use pbs_api_types::{ApiToken, Authid, User, Userid};

use crate::ConfigVersionCache;

use crate::{open_backup_lockfile, replace_backup_config, BackupLockGuard};

lazy_static! {
    pub static ref CONFIG: SectionConfig = init();
}

fn init() -> SectionConfig {
    let mut config = SectionConfig::new(&Authid::API_SCHEMA);

    let user_schema = match User::API_SCHEMA {
        Schema::Object(ref user_schema) => user_schema,
        _ => unreachable!(),
    };
    let user_plugin =
        SectionConfigPlugin::new("user".to_string(), Some("userid".to_string()), user_schema);
    config.register_plugin(user_plugin);

    let token_schema = match ApiToken::API_SCHEMA {
        Schema::Object(ref token_schema) => token_schema,
        _ => unreachable!(),
    };
    let token_plugin = SectionConfigPlugin::new(
        "token".to_string(),
        Some("tokenid".to_string()),
        token_schema,
    );
    config.register_plugin(token_plugin);

    config
}

pub const USER_CFG_FILENAME: &str = "/etc/proxmox-backup/user.cfg";
pub const USER_CFG_LOCKFILE: &str = "/etc/proxmox-backup/.user.lck";

/// Get exclusive lock
pub fn lock_config() -> Result<BackupLockGuard, Error> {
    open_backup_lockfile(USER_CFG_LOCKFILE, None, true)
}

pub fn config() -> Result<(SectionConfigData, [u8; 32]), Error> {
    let content =
        proxmox_sys::fs::file_read_optional_string(USER_CFG_FILENAME)?.unwrap_or_default();

    let digest = openssl::sha::sha256(content.as_bytes());
    let mut data = CONFIG.parse(USER_CFG_FILENAME, &content)?;

    if data.sections.get("root@pam").is_none() {
        let user: User = User {
            userid: Userid::root_userid().clone(),
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
        static ref CACHED_CONFIG: RwLock<ConfigCache> = RwLock::new(ConfigCache {
            data: None,
            last_mtime: 0,
            last_mtime_nsec: 0
        });
    }

    let stat = match nix::sys::stat::stat(USER_CFG_FILENAME) {
        Ok(stat) => Some(stat),
        Err(nix::errno::Errno::ENOENT) => None,
        Err(err) => bail!("unable to stat '{}' - {}", USER_CFG_FILENAME, err),
    };

    {
        // limit scope
        let cache = CACHED_CONFIG.read().unwrap();
        if let Some(ref config) = cache.data {
            if let Some(stat) = stat {
                if stat.st_mtime == cache.last_mtime && stat.st_mtime_nsec == cache.last_mtime_nsec
                {
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
    let raw = CONFIG.write(USER_CFG_FILENAME, config)?;
    replace_backup_config(USER_CFG_FILENAME, raw.as_bytes())?;

    // increase user version
    // We use this in CachedUserInfo
    let version_cache = ConfigVersionCache::new()?;
    version_cache.increase_user_cache_generation();

    Ok(())
}

/// Only exposed for testing
#[doc(hidden)]
pub fn test_cfg_from_str(raw: &str) -> Result<(SectionConfigData, [u8; 32]), Error> {
    let cfg = init();
    let parsed = cfg.parse("test_user_cfg", raw)?;

    Ok((parsed, [0; 32]))
}

// shell completion helper
pub fn complete_userid(_arg: &str, _param: &HashMap<String, String>) -> Vec<String> {
    match config() {
        Ok((data, _digest)) => data
            .sections
            .iter()
            .filter_map(|(id, (section_type, _))| {
                if section_type == "user" {
                    Some(id.to_string())
                } else {
                    None
                }
            })
            .collect(),
        Err(_) => Vec::new(),
    }
}

// shell completion helper
pub fn complete_authid(_arg: &str, _param: &HashMap<String, String>) -> Vec<String> {
    match config() {
        Ok((data, _digest)) => data.sections.keys().map(|id| id.to_string()).collect(),
        Err(_) => vec![],
    }
}

// shell completion helper
pub fn complete_token_name(_arg: &str, param: &HashMap<String, String>) -> Vec<String> {
    let data = match config() {
        Ok((data, _digest)) => data,
        Err(_) => return Vec::new(),
    };

    match param.get("userid") {
        Some(userid) => {
            let user = data.lookup::<User>("user", userid);
            let tokens = data.convert_to_typed_array("token");
            match (user, tokens) {
                (Ok(_), Ok(tokens)) => tokens
                    .into_iter()
                    .filter_map(|token: ApiToken| {
                        let tokenid = token.tokenid;
                        if tokenid.is_token() && tokenid.user() == userid {
                            Some(tokenid.tokenname().unwrap().as_str().to_string())
                        } else {
                            None
                        }
                    })
                    .collect(),
                _ => vec![],
            }
        }
        None => vec![],
    }
}
