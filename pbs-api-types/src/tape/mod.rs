//! Types for tape backup API

mod device;
pub use device::*;

mod changer;
pub use changer::*;

mod drive;
pub use drive::*;

mod media_pool;
pub use media_pool::*;

mod media_status;
pub use media_status::*;

mod media_location;

pub use media_location::*;

mod media;
pub use media::*;

use serde::{Deserialize, Serialize};

use proxmox_schema::{api, const_regex, ApiStringFormat, Schema, StringSchema};
use proxmox_uuid::Uuid;

use crate::{BackupType, BACKUP_ID_SCHEMA, FINGERPRINT_SHA256_FORMAT};

const_regex! {
    pub TAPE_RESTORE_SNAPSHOT_REGEX = concat!(r"^", PROXMOX_SAFE_ID_REGEX_STR!(), r":(?:", BACKUP_NS_PATH_RE!(),")?", SNAPSHOT_PATH_REGEX_STR!(), r"$");
}

pub const TAPE_RESTORE_SNAPSHOT_FORMAT: ApiStringFormat =
    ApiStringFormat::Pattern(&TAPE_RESTORE_SNAPSHOT_REGEX);

pub const TAPE_ENCRYPTION_KEY_FINGERPRINT_SCHEMA: Schema =
    StringSchema::new("Tape encryption key fingerprint (sha256).")
        .format(&FINGERPRINT_SHA256_FORMAT)
        .schema();

pub const TAPE_RESTORE_SNAPSHOT_SCHEMA: Schema =
    StringSchema::new("A snapshot in the format: 'store:[ns/namespace/...]type/id/time")
        .format(&TAPE_RESTORE_SNAPSHOT_FORMAT)
        .type_text("store:[ns/namespace/...]type/id/time")
        .schema();

#[api(
    properties: {
        pool: {
            schema: MEDIA_POOL_NAME_SCHEMA,
            optional: true,
        },
        "label-text": {
            schema: MEDIA_LABEL_SCHEMA,
            optional: true,
        },
        "media": {
            schema: MEDIA_UUID_SCHEMA,
            optional: true,
        },
        "media-set": {
            schema: MEDIA_SET_UUID_SCHEMA,
            optional: true,
        },
        "backup-type": {
            type: BackupType,
            optional: true,
        },
        "backup-id": {
            schema: BACKUP_ID_SCHEMA,
            optional: true,
        },
    },
)]
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
/// Content list filter parameters
pub struct MediaContentListFilter {
    pub pool: Option<String>,
    pub label_text: Option<String>,
    pub media: Option<Uuid>,
    pub media_set: Option<Uuid>,
    pub backup_type: Option<BackupType>,
    pub backup_id: Option<String>,
}
