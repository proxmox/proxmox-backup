//! Exports configuration data from the build system

/// The configured configuration directory
pub const CONFIGDIR: &str = "/etc/proxmox-backup";
pub const JS_DIR: &str = "/usr/share/javascript/proxmox-backup";

#[macro_export]
macro_rules! PROXMOX_BACKUP_RUN_DIR_M { () => ("/run/proxmox-backup") }

#[macro_export]
macro_rules! PROXMOX_BACKUP_LOG_DIR_M { () => ("/var/log/proxmox-backup") }

/// namespaced directory for in-memory (tmpfs) run state
pub const PROXMOX_BACKUP_RUN_DIR: &str = PROXMOX_BACKUP_RUN_DIR_M!();

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

/// Prepend configuration directory to a file name
///
/// This is a simply way to get the full path for configuration files.
/// #### Example:
/// ```
/// # #[macro_use] extern crate proxmox_backup;
/// let cert_path = configdir!("/proxy.pfx");
/// ```
#[macro_export]
macro_rules! configdir {
    ($subdir:expr) => (concat!("/etc/proxmox-backup", $subdir))
}

/// Prepend the run directory to a file name.
///
/// This is a simply way to get the full path for files in `/run`.
#[macro_export]
macro_rules! rundir {
    ($subdir:expr) => {
        concat!(PROXMOX_BACKUP_RUN_DIR_M!(), $subdir)
    };
}
