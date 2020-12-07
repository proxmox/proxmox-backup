use ::serde::{Deserialize, Serialize};

use proxmox::api::api;

#[api()]
#[derive(Debug,Serialize,Deserialize)]
#[serde(rename_all = "kebab-case")]
/// Kind of devive
pub enum DeviceKind {
    /// Tape changer (Autoloader, Robot)
    Changer,
    /// Normal SCSI tape device
    Tape,
}

#[api(
    properties: {
        kind: {
            type: DeviceKind,
        },
    },
)]
#[derive(Debug,Serialize,Deserialize)]
/// Tape device information
pub struct TapeDeviceInfo {
    pub kind: DeviceKind,
    /// Path to the linux device node
    pub path: String,
    /// Serial number (autodetected)
    pub serial: String,
    /// Vendor (autodetected)
    pub vendor: String,
    /// Model (autodetected)
    pub model: String,
    /// Device major number
    pub major: u32,
    /// Device minor number
    pub minor: u32,
}
