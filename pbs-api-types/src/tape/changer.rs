//! Types for tape changer API

use serde::{Deserialize, Serialize};

use proxmox_schema::{
    api, ApiStringFormat, ArraySchema, IntegerSchema, Schema, StringSchema, Updater,
};

use crate::{OptionalDeviceIdentification, PROXMOX_SAFE_ID_FORMAT};

pub const CHANGER_NAME_SCHEMA: Schema = StringSchema::new("Tape Changer Identifier.")
    .format(&PROXMOX_SAFE_ID_FORMAT)
    .min_length(3)
    .max_length(32)
    .schema();

pub const SCSI_CHANGER_PATH_SCHEMA: Schema =
    StringSchema::new("Path to Linux generic SCSI device (e.g. '/dev/sg4')").schema();

pub const MEDIA_LABEL_SCHEMA: Schema = StringSchema::new("Media Label/Barcode.")
    .format(&PROXMOX_SAFE_ID_FORMAT)
    .min_length(2)
    .max_length(32)
    .schema();

pub const SLOT_ARRAY_SCHEMA: Schema = ArraySchema::new(
    "Slot list.",
    &IntegerSchema::new("Slot number").minimum(1).schema(),
)
.schema();

pub const EXPORT_SLOT_LIST_SCHEMA: Schema = StringSchema::new(
    "\
A list of slot numbers, comma separated. Those slots are reserved for
Import/Export, i.e. any media in those slots are considered to be
'offline'.
",
)
.format(&ApiStringFormat::PropertyString(&SLOT_ARRAY_SCHEMA))
.schema();

#[api(
    properties: {
        name: {
            schema: CHANGER_NAME_SCHEMA,
        },
        path: {
            schema: SCSI_CHANGER_PATH_SCHEMA,
        },
        "export-slots": {
            schema: EXPORT_SLOT_LIST_SCHEMA,
            optional: true,
        },
        "eject-before-unload": {
            optional: true,
            default: false,
        }
    },
)]
#[derive(Serialize, Deserialize, Updater)]
#[serde(rename_all = "kebab-case")]
/// SCSI tape changer
pub struct ScsiTapeChanger {
    #[updater(skip)]
    pub name: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub export_slots: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// if set to true, tapes are ejected manually before unloading
    pub eject_before_unload: Option<bool>,
}

#[api(
    properties: {
        config: {
            type: ScsiTapeChanger,
        },
        info: {
            type: OptionalDeviceIdentification,
        },
    },
)]
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
/// Changer config with optional device identification attributes
pub struct ChangerListEntry {
    #[serde(flatten)]
    pub config: ScsiTapeChanger,
    #[serde(flatten)]
    pub info: OptionalDeviceIdentification,
}

#[api()]
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
/// Mtx Entry Kind
pub enum MtxEntryKind {
    /// Drive
    Drive,
    /// Slot
    Slot,
    /// Import/Export Slot
    ImportExport,
}

#[api(
    properties: {
        "entry-kind": {
            type: MtxEntryKind,
        },
        "label-text": {
            schema: MEDIA_LABEL_SCHEMA,
            optional: true,
        },
    },
)]
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
/// Mtx Status Entry
pub struct MtxStatusEntry {
    pub entry_kind: MtxEntryKind,
    /// The ID of the slot or drive
    pub entry_id: u64,
    /// The media label (volume tag) if the slot/drive is full
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label_text: Option<String>,
    /// The slot the drive was loaded from
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loaded_slot: Option<u64>,
    /// The current state of the drive
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
}
