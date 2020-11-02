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

/// logfile for all API reuests handled by the proxy and privileged API daemons
pub const API_ACCESS_LOG_FN: &str = concat!(PROXMOX_BACKUP_LOG_DIR_M!(), "/api/access.log");

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
