//! Types for tape drive API
use std::convert::TryFrom;

use anyhow::{bail, Error};
use serde::{Deserialize, Serialize};

use proxmox::api::{
    api,
    schema::{Schema, IntegerSchema, StringSchema},
};

use crate::api2::types::{
    PROXMOX_SAFE_ID_FORMAT,
    CHANGER_NAME_SCHEMA,
};

pub const DRIVE_NAME_SCHEMA: Schema = StringSchema::new("Drive Identifier.")
    .format(&PROXMOX_SAFE_ID_FORMAT)
    .min_length(3)
    .max_length(32)
    .schema();

pub const LINUX_DRIVE_PATH_SCHEMA: Schema = StringSchema::new(
    "The path to a LINUX non-rewinding SCSI tape device (i.e. '/dev/nst0')")
    .schema();

pub const CHANGER_DRIVE_ID_SCHEMA: Schema = IntegerSchema::new(
    "Associated changer drive number (requires option changer)")
    .minimum(0)
    .maximum(8)
    .default(0)
    .schema();

#[api(
    properties: {
        name: {
            schema: DRIVE_NAME_SCHEMA,
        }
    }
)]
#[derive(Serialize,Deserialize)]
/// Simulate tape drives (only for test and debug)
#[serde(rename_all = "kebab-case")]
pub struct VirtualTapeDrive {
    pub name: String,
    /// Path to directory
    pub path: String,
    /// Virtual tape size
    #[serde(skip_serializing_if="Option::is_none")]
    pub max_size: Option<usize>,
}

#[api(
    properties: {
        name: {
            schema: DRIVE_NAME_SCHEMA,
        },
        path: {
            schema: LINUX_DRIVE_PATH_SCHEMA,
        },
        changer: {
            schema: CHANGER_NAME_SCHEMA,
            optional: true,
        },
        "changer-drive-id": {
            schema: CHANGER_DRIVE_ID_SCHEMA,
            optional: true,
        },
    }
)]
#[derive(Serialize,Deserialize)]
#[serde(rename_all = "kebab-case")]
/// Linux SCSI tape driver
pub struct LinuxTapeDrive {
    pub name: String,
    pub path: String,
    #[serde(skip_serializing_if="Option::is_none")]
    pub changer: Option<String>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub changer_drive_id: Option<u64>,
}

#[api()]
#[derive(Serialize,Deserialize)]
/// Drive list entry
pub struct DriveListEntry {
    /// Drive name
    pub name: String,
    /// Path to the linux device node
    pub path: String,
    /// Associated changer device
    #[serde(skip_serializing_if="Option::is_none")]
    pub changer: Option<String>,
    /// Vendor (autodetected)
    #[serde(skip_serializing_if="Option::is_none")]
    pub vendor: Option<String>,
    /// Model (autodetected)
    #[serde(skip_serializing_if="Option::is_none")]
    pub model: Option<String>,
    /// Serial number (autodetected)
    #[serde(skip_serializing_if="Option::is_none")]
    pub serial: Option<String>,
}

#[api()]
#[derive(Serialize,Deserialize)]
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
#[derive(Serialize,Deserialize,Copy,Clone,Debug)]
pub enum TapeDensity {
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
}

impl TryFrom<u8> for TapeDensity {
    type Error = Error;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        let density = match value {
            0x40 => TapeDensity::LTO1,
            0x42 => TapeDensity::LTO2,
            0x44 => TapeDensity::LTO3,
            0x46 => TapeDensity::LTO4,
            0x58 => TapeDensity::LTO5,
            0x5a => TapeDensity::LTO6,
            0x5c => TapeDensity::LTO7,
            0x5d => TapeDensity::LTO7M8,
            0x5e => TapeDensity::LTO8,
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
#[derive(Serialize,Deserialize)]
#[serde(rename_all = "kebab-case")]
/// Drive/Media status for Linux SCSI drives.
///
/// Media related data is optional - only set if there is a medium
/// loaded.
pub struct LinuxDriveAndMediaStatus {
    /// Block size (0 is variable size)
    pub blocksize: u32,
    /// Tape density
    #[serde(skip_serializing_if="Option::is_none")]
    pub density: Option<TapeDensity>,
    /// Status flags
    pub status: String,
    /// Current file number
    #[serde(skip_serializing_if="Option::is_none")]
    pub file_number: Option<u32>,
    /// Current block number
    #[serde(skip_serializing_if="Option::is_none")]
    pub block_number: Option<u32>,
    /// Medium Manufacture Date (epoch)
    #[serde(skip_serializing_if="Option::is_none")]
    pub manufactured: Option<i64>,
    /// Total Bytes Read in Medium Life
    #[serde(skip_serializing_if="Option::is_none")]
    pub bytes_read: Option<u64>,
    /// Total Bytes Written in Medium Life
    #[serde(skip_serializing_if="Option::is_none")]
    pub bytes_written: Option<u64>,
}
