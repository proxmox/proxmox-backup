use ::serde::{Deserialize, Serialize};

use proxmox::{
    api::api,
    tools::Uuid,
};

use crate::api2::types::{
    MEDIA_UUID_SCHEMA,
    MEDIA_SET_UUID_SCHEMA,
    MediaStatus,
    MediaLocation,
};

#[api(
    properties: {
        location: {
            type: MediaLocation,
        },
        status: {
            type: MediaStatus,
        },
        uuid: {
            schema: MEDIA_UUID_SCHEMA,
        },
        "media-set-uuid": {
            schema: MEDIA_SET_UUID_SCHEMA,
            optional: true,
        },
    },
)]
#[derive(Serialize,Deserialize)]
#[serde(rename_all = "kebab-case")]
/// Media list entry
pub struct MediaListEntry {
    /// Media label text (or Barcode)
    pub label_text: String,
    pub uuid: Uuid,
    /// Creation time stamp
    pub ctime: i64,
    pub location: MediaLocation,
    pub status: MediaStatus,
    /// Expired flag
    pub expired: bool,
    /// Catalog status OK
    pub catalog: bool,
    /// Media set name
    #[serde(skip_serializing_if="Option::is_none")]
    pub media_set_name: Option<String>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub media_set_uuid: Option<Uuid>,
    /// Media set seq_nr
    #[serde(skip_serializing_if="Option::is_none")]
    pub seq_nr: Option<u64>,
    /// MediaSet creation time stamp
    #[serde(skip_serializing_if="Option::is_none")]
    pub media_set_ctime: Option<i64>,
    /// Media Pool
    #[serde(skip_serializing_if="Option::is_none")]
    pub pool: Option<String>,
}

#[api(
    properties: {
        uuid: {
            schema: MEDIA_UUID_SCHEMA,
        },
        "media-set-uuid": {
            schema: MEDIA_SET_UUID_SCHEMA,
            optional: true,
        },
    },
)]
#[derive(Serialize,Deserialize)]
#[serde(rename_all = "kebab-case")]
/// Media label info
pub struct MediaIdFlat {
    /// Unique ID
    pub uuid: Uuid,
    /// Media label text (or Barcode)
    pub label_text: String,
    /// Creation time stamp
    pub ctime: i64,
    // All MediaSet properties are optional here
    /// MediaSet Pool
    #[serde(skip_serializing_if="Option::is_none")]
    pub pool: Option<String>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub media_set_uuid: Option<Uuid>,
    /// MediaSet media sequence number
    #[serde(skip_serializing_if="Option::is_none")]
    pub seq_nr: Option<u64>,
    /// MediaSet Creation time stamp
    #[serde(skip_serializing_if="Option::is_none")]
    pub media_set_ctime: Option<i64>,
    /// Encryption key fingerprint
    #[serde(skip_serializing_if="Option::is_none")]
    pub encryption_key_fingerprint: Option<String>,
}

#[api(
    properties: {
        uuid: {
            schema: MEDIA_UUID_SCHEMA,
            optional: true,
        },
    },
)]
#[derive(Serialize,Deserialize)]
#[serde(rename_all = "kebab-case")]
/// Label with optional Uuid
pub struct LabelUuidMap {
    /// Changer label text (or Barcode)
    pub label_text: String,
    /// Associated Uuid (if any)
    pub uuid: Option<Uuid>,
}

#[api(
    properties: {
        uuid: {
            schema: MEDIA_UUID_SCHEMA,
        },
        "media-set-uuid": {
            schema: MEDIA_SET_UUID_SCHEMA,
        },
    },
)]
#[derive(Serialize,Deserialize)]
#[serde(rename_all = "kebab-case")]
/// Media content list entry
pub struct MediaContentEntry {
    /// Media label text (or Barcode)
    pub label_text: String,
    /// Media Uuid
    pub uuid: Uuid,
    /// Media set name
    pub media_set_name: String,
    /// Media set uuid
    pub media_set_uuid: Uuid,
    /// MediaSet Creation time stamp
    pub media_set_ctime: i64,
    /// Media set seq_nr
    pub seq_nr: u64,
    /// Media Pool
    pub pool: String,
    /// Datastore Name
    pub store: String,
    /// Backup snapshot
    pub snapshot: String,
    /// Snapshot creation time (epoch)
    pub backup_time: i64,
}
