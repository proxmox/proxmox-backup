//! Types for tape backup API
//!
use serde::{Deserialize, Serialize};

use proxmox::api::{
    api,
    schema::{Schema, StringSchema},
};

use super::PROXMOX_SAFE_ID_FORMAT;

pub const DRIVE_ID_SCHEMA: Schema = StringSchema::new("Drive Identifier.")
    .format(&PROXMOX_SAFE_ID_FORMAT)
    .min_length(3)
    .max_length(32)
    .schema();

pub const CHANGER_ID_SCHEMA: Schema = StringSchema::new("Tape Changer Identifier.")
    .format(&PROXMOX_SAFE_ID_FORMAT)
    .min_length(3)
    .max_length(32)
    .schema();

pub const LINUX_DRIVE_PATH_SCHEMA: Schema = StringSchema::new(
    "The path to a LINUX non-rewinding SCSI tape device (i.e. '/dev/nst0')")
    .schema();

pub const SCSI_CHANGER_PATH_SCHEMA: Schema = StringSchema::new(
    "Path to Linux generic SCSI device (i.e. '/dev/sg4')")
    .schema();

#[api(
    properties: {
        name: {
            schema: DRIVE_ID_SCHEMA,
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
            schema: DRIVE_ID_SCHEMA,
        },
        path: {
            schema: LINUX_DRIVE_PATH_SCHEMA,
        },
        changer: {
            schema: CHANGER_ID_SCHEMA,
            optional: true,
        }
    }
)]
#[derive(Serialize,Deserialize)]
/// Linux SCSI tape driver
pub struct LinuxTapeDrive {
    pub name: String,
    pub path: String,
    /// Associated changer device
    #[serde(skip_serializing_if="Option::is_none")]
    pub changer: Option<String>,
}

#[api(
    properties: {
        name: {
            schema: CHANGER_ID_SCHEMA,
        },
        path: {
            schema: SCSI_CHANGER_PATH_SCHEMA,
        },
    }
)]
#[derive(Serialize,Deserialize)]
/// SCSI tape changer
pub struct ScsiTapeChanger {
    pub name: String,
    pub path: String,
}
