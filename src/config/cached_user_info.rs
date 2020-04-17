//! Cached user info for fast ACL permission checks

use std::sync::Arc;

use failure::*;

use proxmox::api::section_config::SectionConfigData;
use proxmox::api::UserInformation;

use super::acl::{AclTree, ROLE_NAMES};
use super::user::User;

/// Cache User/Group/Acl configuration data for fast permission tests
pub struct CachedUserInfo {
    user_cfg: Arc<SectionConfigData>,
    acl_tree: Arc<AclTree>,
}

impl CachedUserInfo {

    /// Creates a new instance.
    pub fn new() -> Result<Self, Error> {
        Ok(CachedUserInfo {
            user_cfg: super::user::cached_config()?,
            acl_tree: super::acl::cached_config()?,
        })
    }

    /// Test if a user account is enabled and not expired
    pub fn is_active_user(&self, userid: &str) -> bool {
        if let Ok(info) = self.user_cfg.lookup::<User>("user", &userid) {
            if !info.enable.unwrap_or(true) {
                return false;
            }
            if let Some(expire) = info.expire {
                if expire > 0 {
                    let now = unsafe { libc::time(std::ptr::null_mut()) };
                    if expire <= now {
                        return false;
                    }
                }
            }
            return true;
        } else {
            return false;
        }
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
