pub const CONFIGDIR: &'static str = env!("PROXMOX_CONFIGDIR");

#[macro_export]
macro_rules! configdir {
    ($subdir:expr) => (concat!(env!("PROXMOX_CONFIGDIR"), $subdir))
}
