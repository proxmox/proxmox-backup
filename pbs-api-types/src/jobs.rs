use serde::{Deserialize, Serialize};

use proxmox::const_regex;

use proxmox::api::{api, schema::*};

use crate::{
    Userid, Authid, REMOTE_ID_SCHEMA, DRIVE_NAME_SCHEMA, MEDIA_POOL_NAME_SCHEMA,
    SINGLE_LINE_COMMENT_SCHEMA, PROXMOX_SAFE_ID_FORMAT, DATASTORE_SCHEMA,
};

const_regex!{

    /// Regex for verification jobs 'DATASTORE:ACTUAL_JOB_ID'
    pub VERIFICATION_JOB_WORKER_ID_REGEX = concat!(r"^(", PROXMOX_SAFE_ID_REGEX_STR!(), r"):");
    /// Regex for sync jobs 'REMOTE:REMOTE_DATASTORE:LOCAL_DATASTORE:ACTUAL_JOB_ID'
    pub SYNC_JOB_WORKER_ID_REGEX = concat!(r"^(", PROXMOX_SAFE_ID_REGEX_STR!(), r"):(", PROXMOX_SAFE_ID_REGEX_STR!(), r"):(", PROXMOX_SAFE_ID_REGEX_STR!(), r"):");
}

pub const JOB_ID_SCHEMA: Schema = StringSchema::new("Job ID.")
    .format(&PROXMOX_SAFE_ID_FORMAT)
    .min_length(3)
    .max_length(32)
    .schema();

pub const SYNC_SCHEDULE_SCHEMA: Schema = StringSchema::new(
    "Run sync job at specified schedule.")
    .format(&ApiStringFormat::VerifyFn(proxmox_systemd::time::verify_calendar_event))
    .type_text("<calendar-event>")
    .schema();

pub const GC_SCHEDULE_SCHEMA: Schema = StringSchema::new(
    "Run garbage collection job at specified schedule.")
    .format(&ApiStringFormat::VerifyFn(proxmox_systemd::time::verify_calendar_event))
    .type_text("<calendar-event>")
    .schema();

pub const PRUNE_SCHEDULE_SCHEMA: Schema = StringSchema::new(
    "Run prune job at specified schedule.")
    .format(&ApiStringFormat::VerifyFn(proxmox_systemd::time::verify_calendar_event))
    .type_text("<calendar-event>")
    .schema();

pub const VERIFICATION_SCHEDULE_SCHEMA: Schema = StringSchema::new(
    "Run verify job at specified schedule.")
    .format(&ApiStringFormat::VerifyFn(proxmox_systemd::time::verify_calendar_event))
    .type_text("<calendar-event>")
    .schema();

pub const REMOVE_VANISHED_BACKUPS_SCHEMA: Schema = BooleanSchema::new(
    "Delete vanished backups. This remove the local copy if the remote backup was deleted.")
    .default(true)
    .schema();

#[api(
    properties: {
        "next-run": {
            description: "Estimated time of the next run (UNIX epoch).",
            optional: true,
            type: Integer,
        },
        "last-run-state": {
            description: "Result of the last run.",
            optional: true,
            type: String,
        },
        "last-run-upid": {
            description: "Task UPID of the last run.",
            optional: true,
            type: String,
        },
        "last-run-endtime": {
            description: "Endtime of the last run.",
            optional: true,
            type: Integer,
        },
    }
)]
#[derive(Serialize,Deserialize,Default)]
#[serde(rename_all="kebab-case")]
/// Job Scheduling Status
pub struct JobScheduleStatus {
    #[serde(skip_serializing_if="Option::is_none")]
    pub next_run: Option<i64>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub last_run_state: Option<String>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub last_run_upid: Option<String>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub last_run_endtime: Option<i64>,
}

#[api()]
#[derive(Debug, Copy, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
/// When do we send notifications
pub enum Notify {
    /// Never send notification
    Never,
    /// Send notifications for failed and successful jobs
    Always,
    /// Send notifications for failed jobs only
    Error,
}

#[api(
    properties: {
        gc: {
            type: Notify,
            optional: true,
        },
        verify: {
            type: Notify,
            optional: true,
        },
        sync: {
            type: Notify,
            optional: true,
        },
    },
)]
#[derive(Debug, Serialize, Deserialize)]
/// Datastore notify settings
pub struct DatastoreNotify {
    /// Garbage collection settings
    pub gc: Option<Notify>,
    /// Verify job setting
    pub verify: Option<Notify>,
    /// Sync job setting
    pub sync: Option<Notify>,
}

pub const DATASTORE_NOTIFY_STRING_SCHEMA: Schema = StringSchema::new(
    "Datastore notification setting")
    .format(&ApiStringFormat::PropertyString(&DatastoreNotify::API_SCHEMA))
    .schema();

pub const IGNORE_VERIFIED_BACKUPS_SCHEMA: Schema = BooleanSchema::new(
    "Do not verify backups that are already verified if their verification is not outdated.")
    .default(true)
    .schema();

pub const VERIFICATION_OUTDATED_AFTER_SCHEMA: Schema = IntegerSchema::new(
    "Days after that a verification becomes outdated")
    .minimum(1)
    .schema();

#[api(
    properties: {
        id: {
            schema: JOB_ID_SCHEMA,
        },
        store: {
            schema: DATASTORE_SCHEMA,
        },
        "ignore-verified": {
            optional: true,
            schema: IGNORE_VERIFIED_BACKUPS_SCHEMA,
        },
        "outdated-after": {
            optional: true,
            schema: VERIFICATION_OUTDATED_AFTER_SCHEMA,
        },
        comment: {
            optional: true,
            schema: SINGLE_LINE_COMMENT_SCHEMA,
        },
        schedule: {
            optional: true,
            schema: VERIFICATION_SCHEDULE_SCHEMA,
        },
    }
)]
#[derive(Serialize,Deserialize,Updater)]
#[serde(rename_all="kebab-case")]
/// Verification Job
pub struct VerificationJobConfig {
    /// unique ID to address this job
    #[updater(skip)]
    pub id: String,
    /// the datastore ID this verificaiton job affects
    pub store: String,
    #[serde(skip_serializing_if="Option::is_none")]
    /// if not set to false, check the age of the last snapshot verification to filter
    /// out recent ones, depending on 'outdated_after' configuration.
    pub ignore_verified: Option<bool>,
    #[serde(skip_serializing_if="Option::is_none")]
    /// Reverify snapshots after X days, never if 0. Ignored if 'ignore_verified' is false.
    pub outdated_after: Option<i64>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub comment: Option<String>,
    #[serde(skip_serializing_if="Option::is_none")]
    /// when to schedule this job in calendar event notation
    pub schedule: Option<String>,
}

#[api(
    properties: {
        config: {
            type: VerificationJobConfig,
        },
        status: {
            type: JobScheduleStatus,
        },
    },
)]
#[derive(Serialize,Deserialize)]
#[serde(rename_all="kebab-case")]
/// Status of Verification Job
pub struct VerificationJobStatus {
    #[serde(flatten)]
    pub config: VerificationJobConfig,
    #[serde(flatten)]
    pub status: JobScheduleStatus,
}

#[api(
    properties: {
        store: {
           schema: DATASTORE_SCHEMA,
        },
        pool: {
            schema: MEDIA_POOL_NAME_SCHEMA,
        },
        drive: {
            schema: DRIVE_NAME_SCHEMA,
        },
        "eject-media": {
            description: "Eject media upon job completion.",
            type: bool,
            optional: true,
        },
        "export-media-set": {
            description: "Export media set upon job completion.",
            type: bool,
            optional: true,
        },
        "latest-only": {
            description: "Backup latest snapshots only.",
            type: bool,
            optional: true,
        },
        "notify-user": {
            optional: true,
            type: Userid,
        },
    }
)]
#[derive(Serialize,Deserialize,Clone,Updater)]
#[serde(rename_all="kebab-case")]
/// Tape Backup Job Setup
pub struct TapeBackupJobSetup {
    pub store: String,
    pub pool: String,
    pub drive: String,
    #[serde(skip_serializing_if="Option::is_none")]
    pub eject_media: Option<bool>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub export_media_set: Option<bool>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub latest_only: Option<bool>,
    /// Send job email notification to this user
    #[serde(skip_serializing_if="Option::is_none")]
    pub notify_user: Option<Userid>,
}

#[api(
    properties: {
        id: {
            schema: JOB_ID_SCHEMA,
        },
        setup: {
            type: TapeBackupJobSetup,
        },
        comment: {
            optional: true,
            schema: SINGLE_LINE_COMMENT_SCHEMA,
        },
        schedule: {
            optional: true,
            schema: SYNC_SCHEDULE_SCHEMA,
        },
    }
)]
#[derive(Serialize,Deserialize,Clone,Updater)]
#[serde(rename_all="kebab-case")]
/// Tape Backup Job
pub struct TapeBackupJobConfig {
    #[updater(skip)]
    pub id: String,
    #[serde(flatten)]
    pub setup: TapeBackupJobSetup,
    #[serde(skip_serializing_if="Option::is_none")]
    pub comment: Option<String>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub schedule: Option<String>,
}

#[api(
    properties: {
        config: {
            type: TapeBackupJobConfig,
        },
        status: {
            type: JobScheduleStatus,
        },
    },
)]
#[derive(Serialize,Deserialize)]
#[serde(rename_all="kebab-case")]
/// Status of Tape Backup Job
pub struct TapeBackupJobStatus {
    #[serde(flatten)]
    pub config: TapeBackupJobConfig,
    #[serde(flatten)]
    pub status: JobScheduleStatus,
    /// Next tape used (best guess)
    #[serde(skip_serializing_if="Option::is_none")]
    pub next_media_label: Option<String>,
}

#[api(
    properties: {
        id: {
            schema: JOB_ID_SCHEMA,
        },
        store: {
           schema: DATASTORE_SCHEMA,
        },
        "owner": {
            type: Authid,
            optional: true,
        },
        remote: {
            schema: REMOTE_ID_SCHEMA,
        },
        "remote-store": {
            schema: DATASTORE_SCHEMA,
        },
        "remove-vanished": {
            schema: REMOVE_VANISHED_BACKUPS_SCHEMA,
            optional: true,
        },
        comment: {
            optional: true,
            schema: SINGLE_LINE_COMMENT_SCHEMA,
        },
        schedule: {
            optional: true,
            schema: SYNC_SCHEDULE_SCHEMA,
        },
    }
)]
#[derive(Serialize,Deserialize,Clone,Updater)]
#[serde(rename_all="kebab-case")]
/// Sync Job
pub struct SyncJobConfig {
    #[updater(skip)]
    pub id: String,
    pub store: String,
    #[serde(skip_serializing_if="Option::is_none")]
    pub owner: Option<Authid>,
    pub remote: String,
    pub remote_store: String,
    #[serde(skip_serializing_if="Option::is_none")]
    pub remove_vanished: Option<bool>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub comment: Option<String>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub schedule: Option<String>,
}

#[api(
    properties: {
        config: {
            type: SyncJobConfig,
        },
        status: {
            type: JobScheduleStatus,
        },
    },
)]

#[derive(Serialize,Deserialize)]
#[serde(rename_all="kebab-case")]
/// Status of Sync Job
pub struct SyncJobStatus {
    #[serde(flatten)]
    pub config: SyncJobConfig,
    #[serde(flatten)]
    pub status: JobScheduleStatus,
}
