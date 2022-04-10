use ::serde::{Deserialize, Serialize};

use proxmox_schema::api;

#[api()]
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
/// Optional Device Identification Attributes
pub struct OptionalDeviceIdentification {
    /// Vendor (autodetected)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vendor: Option<String>,
    /// Model (autodetected)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Serial number (autodetected)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub serial: Option<String>,
}

#[api()]
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
/// Kind of device
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
#[derive(Debug, Serialize, Deserialize)]
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
