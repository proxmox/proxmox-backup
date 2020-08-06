//! Cached user info for fast ACL permission checks

use std::sync::{RwLock, Arc};

use anyhow::{Error, bail};

use proxmox::api::section_config::SectionConfigData;
use lazy_static::lazy_static;
use proxmox::api::UserInformation;

use super::acl::{AclTree, ROLE_NAMES, ROLE_ADMIN};
use super::user::User;
use crate::api2::types::Userid;

/// Cache User/Group/Acl configuration data for fast permission tests
pub struct CachedUserInfo {
    user_cfg: Arc<SectionConfigData>,
    acl_tree: Arc<AclTree>,
}

fn now() -> i64 { unsafe { libc::time(std::ptr::null_mut()) } }

struct ConfigCache {
    data: Option<Arc<CachedUserInfo>>,
    last_update: i64,
}

lazy_static! {
    static ref CACHED_CONFIG: RwLock<ConfigCache> = RwLock::new(
        ConfigCache { data: None, last_update: 0 }
    );
}

impl CachedUserInfo {

    /// Returns a cached instance (up to 5 seconds old).
    pub fn new() -> Result<Arc<Self>, Error> {
        let now = now();
        { // limit scope
            let cache = CACHED_CONFIG.read().unwrap();
            if (now - cache.last_update) < 5 {
                if let Some(ref config) = cache.data {
                    return Ok(config.clone());
                }
            }
        }

        let config = Arc::new(CachedUserInfo {
            user_cfg: super::user::cached_config()?,
            acl_tree: super::acl::cached_config()?,
        });

        let mut cache = CACHED_CONFIG.write().unwrap();
        cache.last_update = now;
        cache.data = Some(config.clone());

        Ok(config)
    }

    /// Test if a user account is enabled and not expired
    pub fn is_active_user(&self, userid: &Userid) -> bool {
        if let Ok(info) = self.user_cfg.lookup::<User>("user", userid.as_str()) {
            if !info.enable.unwrap_or(true) {
                return false;
            }
            if let Some(expire) = info.expire {
                if expire > 0 {
                    if expire <= now() {
                        return false;
                    }
                }
            }
            return true;
        } else {
            return false;
        }
    }

    pub fn check_privs(
        &self,
        userid: &Userid,
        path: &[&str],
        required_privs: u64,
        partial: bool,
    ) -> Result<(), Error> {
        let user_privs = self.lookup_privs(&userid, path);
        let allowed = if partial {
            (user_privs & required_privs) != 0
        } else {
            (user_privs & required_privs) == required_privs
        };
        if !allowed {
            // printing the path doesn't leaks any information as long as we
            // always check privilege before resource existence
            bail!("no permissions on '/{}'", path.join("/"));
        }
        Ok(())
    }
}

impl CachedUserInfo {
    pub fn is_superuser(&self, userid: &Userid) -> bool {
        userid == "root@pam"
    }

    pub fn is_group_member(&self, _userid: &Userid, _group: &str) -> bool {
        false
    }

    pub fn lookup_privs(&self, userid: &Userid, path: &[&str]) -> u64 {

        if self.is_superuser(userid) {
            return ROLE_ADMIN;
        }

        let roles = self.acl_tree.roles(userid, path);
        let mut privs: u64 = 0;
        for role in roles {
            if let Some((role_privs, _)) = ROLE_NAMES.get(role.as_str()) {
                privs |= role_privs;
            }
        }
        privs
    }
}

impl UserInformation for CachedUserInfo {
    fn is_superuser(&self, userid: &str) -> bool {
        userid == "root@pam"
    }

    fn is_group_member(&self, _userid: &str, _group: &str) -> bool {
        false
    }

    fn lookup_privs(&self, userid: &str, path: &[&str]) -> u64 {
        match userid.parse::<Userid>() {
            Ok(userid) => Self::lookup_privs(self, &userid, path),
            Err(_) => 0,
        }
    }
}
