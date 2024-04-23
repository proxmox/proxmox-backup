use anyhow::Error;

use proxmox_notify::Config;

use pbs_buildcfg::configdir;

use crate::{open_backup_lockfile, BackupLockGuard};

/// Configuration file location for notification targets/matchers.
pub const NOTIFICATION_CONFIG_PATH: &str = configdir!("/notifications.cfg");

/// Private configuration file location for secrets - only readable by `root`.
pub const NOTIFICATION_PRIV_CONFIG_PATH: &str = configdir!("/notifications-priv.cfg");

/// Lockfile to prevent concurrent write access.
pub const NOTIFICATION_LOCK_FILE: &str = configdir!("/.notifications.lck");

/// Get exclusive lock for `notifications.cfg`
pub fn lock_config() -> Result<BackupLockGuard, Error> {
    open_backup_lockfile(NOTIFICATION_LOCK_FILE, None, true)
}

/// Load notification config.
pub fn config() -> Result<Config, Error> {
    let content =
        proxmox_sys::fs::file_read_optional_string(NOTIFICATION_CONFIG_PATH)?.unwrap_or_default();

    let priv_content = proxmox_sys::fs::file_read_optional_string(NOTIFICATION_PRIV_CONFIG_PATH)?
        .unwrap_or_default();

    Ok(Config::new(&content, &priv_content)?)
}

/// Save notification config.
pub fn save_config(config: Config) -> Result<(), Error> {
    let (cfg, priv_cfg) = config.write()?;
    crate::replace_backup_config(NOTIFICATION_CONFIG_PATH, cfg.as_bytes())?;
    crate::replace_secret_config(NOTIFICATION_PRIV_CONFIG_PATH, priv_cfg.as_bytes())?;

    Ok(())
}
