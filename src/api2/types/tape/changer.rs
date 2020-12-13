//! Types for tape changer API

use serde::{Deserialize, Serialize};

use proxmox::api::{
    api,
    schema::{Schema, StringSchema},
};

use crate::api2::types::PROXMOX_SAFE_ID_FORMAT;

pub const CHANGER_NAME_SCHEMA: Schema = StringSchema::new("Tape Changer Identifier.")
    .format(&PROXMOX_SAFE_ID_FORMAT)
    .min_length(3)
    .max_length(32)
    .schema();

pub const SCSI_CHANGER_PATH_SCHEMA: Schema = StringSchema::new(
    "Path to Linux generic SCSI device (i.e. '/dev/sg4')")
    .schema();

pub const MEDIA_LABEL_SCHEMA: Schema = StringSchema::new("Media Label/Barcode.")
    .format(&PROXMOX_SAFE_ID_FORMAT)
    .min_length(3)
    .max_length(32)
    .schema();

#[api(
    properties: {
        name: {
            schema: CHANGER_NAME_SCHEMA,
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

#[api()]
#[derive(Serialize,Deserialize)]
#[serde(rename_all = "lowercase")]
/// Mtx Entry Kind
pub enum MtxEntryKind {
    /// Drive
    Drive,
    /// Slot
    Slot,
}

#[api(
    properties: {
        "entry-kind": {
            type: MtxEntryKind,
        },
        "changer-id": {
            schema: MEDIA_LABEL_SCHEMA,
            optional: true,
        },
    },
)]
#[derive(Serialize,Deserialize)]
#[serde(rename_all = "kebab-case")]
/// Mtx Status Entry
pub struct MtxStatusEntry {
    pub entry_kind: MtxEntryKind,
    /// The ID of the slot or drive
    pub entry_id: u64,
    /// The media label (volume tag) if the slot/drive is full
    #[serde(skip_serializing_if="Option::is_none")]
    pub changer_id: Option<String>,
    /// The slot the drive was loaded from
    #[serde(skip_serializing_if="Option::is_none")]
    pub loaded_slot: Option<u64>,
}
