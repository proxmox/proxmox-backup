use serde::{Deserialize, Serialize};

use proxmox_schema::*;

const_regex! {
    pub ZPOOL_NAME_REGEX = r"^[a-zA-Z][a-z0-9A-Z\-_.:]+$";
}

pub const ZFS_ASHIFT_SCHEMA: Schema = IntegerSchema::new("Pool sector size exponent.")
    .minimum(9)
    .maximum(16)
    .default(12)
    .schema();

pub const ZPOOL_NAME_SCHEMA: Schema = StringSchema::new("ZFS Pool Name")
    .format(&ApiStringFormat::Pattern(&ZPOOL_NAME_REGEX))
    .schema();

#[api(default: "On")]
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
/// The ZFS compression algorithm to use.
pub enum ZfsCompressionType {
    /// Gnu Zip
    Gzip,
    /// LZ4
    Lz4,
    /// LZJB
    Lzjb,
    /// ZLE
    Zle,
    /// ZStd
    ZStd,
    /// Enable compression using the default algorithm.
    On,
    /// Disable compression.
    Off,
}

#[api()]
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
/// The ZFS RAID level to use.
pub enum ZfsRaidLevel {
    /// Single Disk
    Single,
    /// Mirror
    Mirror,
    /// Raid10
    Raid10,
    /// RaidZ
    RaidZ,
    /// RaidZ2
    RaidZ2,
    /// RaidZ3
    RaidZ3,
}

#[api()]
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
/// zpool list item
pub struct ZpoolListItem {
    /// zpool name
    pub name: String,
    /// Health
    pub health: String,
    /// Total size
    pub size: u64,
    /// Used size
    pub alloc: u64,
    /// Free space
    pub free: u64,
    /// ZFS fragnentation level
    pub frag: u64,
    /// ZFS deduplication ratio
    pub dedup: f64,
}
