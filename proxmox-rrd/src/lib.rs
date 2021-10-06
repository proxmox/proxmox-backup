//! # Simple Round Robin Database files with fixed format
//!
//! ## Features
//!
//! * One file stores a single data source
//! * Small/constant file size (6008 bytes)
//! * Stores avarage and maximum values
//! * Stores data for different time resolution ([RRDTimeFrameResolution](proxmox_rrd_api_types::RRDTimeFrameResolution))

pub mod rrd;

mod cache;
pub use cache::*;

/// RRD data source tyoe
pub enum DST {
    /// Gauge values are stored unmodified.
    Gauge,
    /// Stores the difference to the previous value.
    Derive,
}
