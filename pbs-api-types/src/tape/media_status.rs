use serde::{Deserialize, Serialize};

use proxmox_schema::api;

#[api()]
/// Media status
#[derive(Debug, PartialEq, Eq, Copy, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
/// Media Status
pub enum MediaStatus {
    /// Media is ready to be written
    Writable,
    /// Media is full (contains data)
    Full,
    /// Media is marked as unknown, needs rescan
    Unknown,
    /// Media is marked as damaged
    Damaged,
    /// Media is marked as retired
    Retired,
}
