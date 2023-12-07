//! Types for tape drive API
use anyhow::{bail, Error};
use serde::{Deserialize, Serialize};

use proxmox_schema::{api, IntegerSchema, Schema, StringSchema, Updater};

use crate::{OptionalDeviceIdentification, CHANGER_NAME_SCHEMA, PROXMOX_SAFE_ID_FORMAT};

pub const DRIVE_NAME_SCHEMA: Schema = StringSchema::new("Drive Identifier.")
    .format(&PROXMOX_SAFE_ID_FORMAT)
    .min_length(3)
    .max_length(32)
    .schema();

pub const LTO_DRIVE_PATH_SCHEMA: Schema =
    StringSchema::new("The path to a LTO SCSI-generic tape device (i.e. '/dev/sg0')").schema();

pub const CHANGER_DRIVENUM_SCHEMA: Schema =
    IntegerSchema::new("Associated changer drive number (requires option changer)")
        .minimum(0)
        .maximum(255)
        .default(0)
        .schema();

#[api(
    properties: {
        name: {
            schema: DRIVE_NAME_SCHEMA,
        }
    }
)]
#[derive(Serialize, Deserialize)]
/// Simulate tape drives (only for test and debug)
#[serde(rename_all = "kebab-case")]
pub struct VirtualTapeDrive {
    pub name: String,
    /// Path to directory
    pub path: String,
    /// Virtual tape size
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_size: Option<usize>,
}

#[api(
    properties: {
        name: {
            schema: DRIVE_NAME_SCHEMA,
        },
        path: {
            schema: LTO_DRIVE_PATH_SCHEMA,
        },
        changer: {
            schema: CHANGER_NAME_SCHEMA,
            optional: true,
        },
        "changer-drivenum": {
            schema: CHANGER_DRIVENUM_SCHEMA,
            optional: true,
        },
    }
)]
#[derive(Serialize, Deserialize, Updater, Clone)]
#[serde(rename_all = "kebab-case")]
/// Lto SCSI tape driver
pub struct LtoTapeDrive {
    #[updater(skip)]
    pub name: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub changer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub changer_drivenum: Option<u64>,
}

#[api(
    properties: {
        config: {
            type: LtoTapeDrive,
        },
        info: {
            type: OptionalDeviceIdentification,
        },
    },
)]
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
/// Drive list entry
pub struct DriveListEntry {
    #[serde(flatten)]
    pub config: LtoTapeDrive,
    #[serde(flatten)]
    pub info: OptionalDeviceIdentification,
    /// the state of the drive if locked
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
}

#[api()]
#[derive(Serialize, Deserialize)]
/// Medium auxiliary memory attributes (MAM)
pub struct MamAttribute {
    /// Attribute id
    pub id: u16,
    /// Attribute name
    pub name: String,
    /// Attribute value
    pub value: String,
}

#[api()]
#[derive(Serialize, Deserialize, Copy, Clone, Debug, PartialOrd, PartialEq)]
pub enum TapeDensity {
    /// Unknown (no media loaded)
    Unknown,
    /// LTO1
    LTO1,
    /// LTO2
    LTO2,
    /// LTO3
    LTO3,
    /// LTO4
    LTO4,
    /// LTO5
    LTO5,
    /// LTO6
    LTO6,
    /// LTO7
    LTO7,
    /// LTO7M8
    LTO7M8,
    /// LTO8
    LTO8,
    /// LTO9
    LTO9,
}

impl TryFrom<u8> for TapeDensity {
    type Error = Error;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        let density = match value {
            0x00 => TapeDensity::Unknown,
            0x40 => TapeDensity::LTO1,
            0x42 => TapeDensity::LTO2,
            0x44 => TapeDensity::LTO3,
            0x46 => TapeDensity::LTO4,
            0x58 => TapeDensity::LTO5,
            0x5a => TapeDensity::LTO6,
            0x5c => TapeDensity::LTO7,
            0x5d => TapeDensity::LTO7M8,
            0x5e => TapeDensity::LTO8,
            0x60 => TapeDensity::LTO9,
            _ => bail!("unknown tape density code 0x{:02x}", value),
        };
        Ok(density)
    }
}

#[api(
    properties: {
        density: {
            type: TapeDensity,
            optional: true,
        },
    },
)]
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
/// Drive/Media status for Lto SCSI drives.
///
/// Media related data is optional - only set if there is a medium
/// loaded.
pub struct LtoDriveAndMediaStatus {
    /// Vendor
    pub vendor: String,
    /// Product
    pub product: String,
    /// Revision
    pub revision: String,
    /// Block size (0 is variable size)
    pub blocksize: u32,
    /// Compression enabled
    pub compression: bool,
    /// Drive buffer mode
    pub buffer_mode: u8,
    /// Tape density
    pub density: TapeDensity,
    /// Media is write protected
    #[serde(skip_serializing_if = "Option::is_none")]
    pub write_protect: Option<bool>,
    /// Tape Alert Flags
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alert_flags: Option<String>,
    /// Current file number
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_number: Option<u64>,
    /// Current block number
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_number: Option<u64>,
    /// Medium Manufacture Date (epoch)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manufactured: Option<i64>,
    /// Total Bytes Read in Medium Life
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes_read: Option<u64>,
    /// Total Bytes Written in Medium Life
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes_written: Option<u64>,
    /// Number of mounts for the current volume (i.e., Thread Count)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume_mounts: Option<u64>,
    /// Count of the total number of times the medium has passed over
    /// the head.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub medium_passes: Option<u64>,
    /// Estimated tape wearout factor (assuming max. 16000 end-to-end passes)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub medium_wearout: Option<f64>,
}

#[api()]
/// Volume statistics from SCSI log page 17h
#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Lp17VolumeStatistics {
    /// Volume mounts (thread count)
    pub volume_mounts: u64,
    /// Total data sets written
    pub volume_datasets_written: u64,
    /// Write retries
    pub volume_recovered_write_data_errors: u64,
    /// Total unrecovered write errors
    pub volume_unrecovered_write_data_errors: u64,
    /// Total suspended writes
    pub volume_write_servo_errors: u64,
    /// Total fatal suspended writes
    pub volume_unrecovered_write_servo_errors: u64,
    /// Total datasets read
    pub volume_datasets_read: u64,
    /// Total read retries
    pub volume_recovered_read_errors: u64,
    /// Total unrecovered read errors
    pub volume_unrecovered_read_errors: u64,
    /// Last mount unrecovered write errors
    pub last_mount_unrecovered_write_errors: u64,
    /// Last mount unrecovered read errors
    pub last_mount_unrecovered_read_errors: u64,
    /// Last mount bytes written
    pub last_mount_bytes_written: u64,
    /// Last mount bytes read
    pub last_mount_bytes_read: u64,
    /// Lifetime bytes written
    pub lifetime_bytes_written: u64,
    /// Lifetime bytes read
    pub lifetime_bytes_read: u64,
    /// Last load write compression ratio
    pub last_load_write_compression_ratio: u64,
    /// Last load read compression ratio
    pub last_load_read_compression_ratio: u64,
    /// Medium mount time
    pub medium_mount_time: u64,
    /// Medium ready time
    pub medium_ready_time: u64,
    /// Total native capacity
    pub total_native_capacity: u64,
    /// Total used native capacity
    pub total_used_native_capacity: u64,
    /// Write protect
    pub write_protect: bool,
    /// Volume is WORM
    pub worm: bool,
    /// Beginning of medium passes
    pub beginning_of_medium_passes: u64,
    /// Middle of medium passes
    pub middle_of_tape_passes: u64,
    /// Volume serial number
    pub serial: String,
}
