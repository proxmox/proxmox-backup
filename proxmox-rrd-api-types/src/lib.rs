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
#[derive(Copy, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
/// RRD time frame resolution
pub enum RRDTimeFrameResolution {
    /// Hour
    Hour,
    /// Day
    Day,
    /// Week
    Week,
    /// Month
    Month,
    /// Year
    Year,
    /// Decade (10 years)
    Decade,
}
