//! Exports configuration data from the build system

/// The configured configuration directory
pub const CONFIGDIR: &'static str = env!("PROXMOX_CONFIGDIR");

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
    ($subdir:expr) => (concat!(env!("PROXMOX_CONFIGDIR"), $subdir))
}
