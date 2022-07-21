//! Exports configuration data from the build system

pub const PROXMOX_PKG_VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION_MAJOR"),
    ".",
    env!("CARGO_PKG_VERSION_MINOR"),
);
pub const PROXMOX_PKG_RELEASE: &str = env!("CARGO_PKG_VERSION_PATCH");
pub const PROXMOX_PKG_REPOID: &str = env!("REPOID");

/// The configured configuration directory
pub const CONFIGDIR: &str = "/etc/proxmox-backup";
pub const JS_DIR: &str = "/usr/share/javascript/proxmox-backup";

/// Unix system user used by proxmox-backup-proxy
pub const BACKUP_USER_NAME: &str = "backup";
/// Unix system group used by proxmox-backup-proxy
pub const BACKUP_GROUP_NAME: &str = "backup";

#[macro_export]
macro_rules! PROXMOX_BACKUP_RUN_DIR_M {
    () => {
        "/run/proxmox-backup"
    };
}

#[macro_export]
macro_rules! PROXMOX_BACKUP_STATE_DIR_M {
    () => {
        "/var/lib/proxmox-backup"
    };
}

#[macro_export]
macro_rules! PROXMOX_BACKUP_LOG_DIR_M {
    () => {
        "/var/log/proxmox-backup"
    };
}

#[macro_export]
macro_rules! PROXMOX_BACKUP_CACHE_DIR_M {
    () => {
        "/var/cache/proxmox-backup"
    };
}

#[macro_export]
macro_rules! PROXMOX_BACKUP_FILE_RESTORE_BIN_DIR_M {
    () => {
        "/usr/lib/x86_64-linux-gnu/proxmox-backup/file-restore"
    };
}

/// namespaced directory for in-memory (tmpfs) run state
pub const PROXMOX_BACKUP_RUN_DIR: &str = PROXMOX_BACKUP_RUN_DIR_M!();

/// namespaced directory for persistent state
pub const PROXMOX_BACKUP_STATE_DIR: &str = PROXMOX_BACKUP_STATE_DIR_M!();

/// namespaced directory for persistent logging
pub const PROXMOX_BACKUP_LOG_DIR: &str = PROXMOX_BACKUP_LOG_DIR_M!();

/// logfile for all API requests handled by the proxy and privileged API daemons. Note that not all
/// failed logins can be logged here with full information, use the auth log for that.
pub const API_ACCESS_LOG_FN: &str = concat!(PROXMOX_BACKUP_LOG_DIR_M!(), "/api/access.log");

/// logfile for any failed authentication, via ticket or via token, and new successful ticket
/// creations. This file can be useful for fail2ban.
pub const API_AUTH_LOG_FN: &str = concat!(PROXMOX_BACKUP_LOG_DIR_M!(), "/api/auth.log");

/// the PID filename for the unprivileged proxy daemon
pub const PROXMOX_BACKUP_PROXY_PID_FN: &str = concat!(PROXMOX_BACKUP_RUN_DIR_M!(), "/proxy.pid");

/// the PID filename for the privileged api daemon
pub const PROXMOX_BACKUP_API_PID_FN: &str = concat!(PROXMOX_BACKUP_RUN_DIR_M!(), "/api.pid");

/// filename of the cached initramfs to use for booting single file restore VMs, this file is
/// automatically created by APT hooks
pub const PROXMOX_BACKUP_INITRAMFS_FN: &str =
    concat!(PROXMOX_BACKUP_CACHE_DIR_M!(), "/file-restore-initramfs.img");

/// filename of the cached initramfs to use for debugging single file restore
pub const PROXMOX_BACKUP_INITRAMFS_DBG_FN: &str = concat!(
    PROXMOX_BACKUP_CACHE_DIR_M!(),
    "/file-restore-initramfs-debug.img"
);

/// filename of the kernel to use for booting single file restore VMs
pub const PROXMOX_BACKUP_KERNEL_FN: &str =
    concat!(PROXMOX_BACKUP_FILE_RESTORE_BIN_DIR_M!(), "/bzImage");

pub const PROXMOX_BACKUP_SUBSCRIPTION_FN: &str = configdir!("/subscription");

/// Prepend configuration directory to a file name
///
/// This is a simply way to get the full path for configuration files.
/// #### Example:
/// ```
/// use pbs_buildcfg::configdir;
/// let cert_path = configdir!("/proxy.pfx");
/// ```
#[macro_export]
macro_rules! configdir {
    ($subdir:expr) => {
        concat!("/etc/proxmox-backup", $subdir)
    };
}

/// Prepend the run directory to a file name.
///
/// This is a simply way to get the full path for files in `/run`.
#[macro_export]
macro_rules! rundir {
    ($subdir:expr) => {
        concat!($crate::PROXMOX_BACKUP_RUN_DIR_M!(), $subdir)
    };
}
