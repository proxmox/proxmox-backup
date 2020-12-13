//! Types for tape drive API

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
