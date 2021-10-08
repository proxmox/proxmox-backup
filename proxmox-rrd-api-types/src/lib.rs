use serde::{Deserialize, Serialize};

use proxmox_schema::api;

#[api()]
#[derive(Copy, Clone, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
/// RRD consolidation mode
pub enum RRDMode {
    /// Maximum
    Max,
    /// Average
    Average,
}

#[api()]
#[repr(u64)]
#[derive(Copy, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
/// RRD time frame resolution
pub enum RRDTimeFrameResolution {
    ///  1 min => last 70 minutes
    Hour = 60,
    /// 30 min => last 35 hours
    Day = 60*30,
    /// 3 hours => about 8 days
    Week = 60*180,
    /// 12 hours => last 35 days
    Month = 60*720,
    /// 1 week => last 490 days
    Year = 60*10080,
}
