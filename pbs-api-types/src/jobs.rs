use std::str::FromStr;

use anyhow::bail;
use regex::Regex;
use serde::{Deserialize, Serialize};

use proxmox_schema::*;

use crate::{
    Authid, BackupNamespace, BackupType, RateLimitConfig, Userid, BACKUP_GROUP_SCHEMA,
    BACKUP_NAMESPACE_SCHEMA, DATASTORE_SCHEMA, DRIVE_NAME_SCHEMA, MEDIA_POOL_NAME_SCHEMA,
    NS_MAX_DEPTH_REDUCED_SCHEMA, PROXMOX_SAFE_ID_FORMAT, REMOTE_ID_SCHEMA,
    SINGLE_LINE_COMMENT_SCHEMA,
};

const_regex! {

    /// Regex for verification jobs 'DATASTORE:ACTUAL_JOB_ID'
    pub VERIFICATION_JOB_WORKER_ID_REGEX = concat!(r"^(", PROXMOX_SAFE_ID_REGEX_STR!(), r"):");
    /// Regex for sync jobs '(REMOTE|\-):REMOTE_DATASTORE:LOCAL_DATASTORE:(?:LOCAL_NS_ANCHOR:)ACTUAL_JOB_ID'
    pub SYNC_JOB_WORKER_ID_REGEX = concat!(r"^(", PROXMOX_SAFE_ID_REGEX_STR!(), r"|\-):(", PROXMOX_SAFE_ID_REGEX_STR!(), r"):(", PROXMOX_SAFE_ID_REGEX_STR!(), r")(?::(", BACKUP_NS_RE!(), r"))?:");
}

pub const JOB_ID_SCHEMA: Schema = StringSchema::new("Job ID.")
    .format(&PROXMOX_SAFE_ID_FORMAT)
    .min_length(3)
    .max_length(32)
    .schema();

pub const SYNC_SCHEDULE_SCHEMA: Schema = StringSchema::new("Run sync job at specified schedule.")
    .format(&ApiStringFormat::VerifyFn(
        proxmox_time::verify_calendar_event,
    ))
    .type_text("<calendar-event>")
    .schema();

pub const GC_SCHEDULE_SCHEMA: Schema =
    StringSchema::new("Run garbage collection job at specified schedule.")
        .format(&ApiStringFormat::VerifyFn(
            proxmox_time::verify_calendar_event,
        ))
        .type_text("<calendar-event>")
        .schema();

pub const PRUNE_SCHEDULE_SCHEMA: Schema = StringSchema::new("Run prune job at specified schedule.")
    .format(&ApiStringFormat::VerifyFn(
        proxmox_time::verify_calendar_event,
    ))
    .type_text("<calendar-event>")
    .schema();

pub const VERIFICATION_SCHEDULE_SCHEMA: Schema =
    StringSchema::new("Run verify job at specified schedule.")
        .format(&ApiStringFormat::VerifyFn(
            proxmox_time::verify_calendar_event,
        ))
        .type_text("<calendar-event>")
        .schema();

pub const REMOVE_VANISHED_BACKUPS_SCHEMA: Schema = BooleanSchema::new(
    "Delete vanished backups. This remove the local copy if the remote backup was deleted.",
)
.default(false)
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
#[derive(Serialize, Deserialize, Default, Clone, PartialEq)]
#[serde(rename_all = "kebab-case")]
/// Job Scheduling Status
pub struct JobScheduleStatus {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_run: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_run_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_run_upid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_run_endtime: Option<i64>,
}

#[api()]
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
        prune: {
            type: Notify,
            optional: true,
        },
    },
)]
#[derive(Debug, Serialize, Deserialize)]
/// Datastore notify settings
pub struct DatastoreNotify {
    /// Garbage collection settings
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gc: Option<Notify>,
    /// Verify job setting
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verify: Option<Notify>,
    /// Sync job setting
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sync: Option<Notify>,
    /// Prune job setting
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prune: Option<Notify>,
}

pub const DATASTORE_NOTIFY_STRING_SCHEMA: Schema = StringSchema::new(
    "Datastore notification setting, enum can be one of 'always', 'never', or 'error'.",
)
.format(&ApiStringFormat::PropertyString(
    &DatastoreNotify::API_SCHEMA,
))
.schema();

pub const IGNORE_VERIFIED_BACKUPS_SCHEMA: Schema = BooleanSchema::new(
    "Do not verify backups that are already verified if their verification is not outdated.",
)
.default(true)
.schema();

pub const VERIFICATION_OUTDATED_AFTER_SCHEMA: Schema =
    IntegerSchema::new("Days after that a verification becomes outdated. (0 is deprecated)'")
        .minimum(0)
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
        ns: {
            optional: true,
            schema: BACKUP_NAMESPACE_SCHEMA,
        },
        "max-depth": {
            optional: true,
            schema: crate::NS_MAX_DEPTH_SCHEMA,
        },
    }
)]
#[derive(Serialize, Deserialize, Updater, Clone, PartialEq)]
#[serde(rename_all = "kebab-case")]
/// Verification Job
pub struct VerificationJobConfig {
    /// unique ID to address this job
    #[updater(skip)]
    pub id: String,
    /// the datastore ID this verification job affects
    pub store: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// if not set to false, check the age of the last snapshot verification to filter
    /// out recent ones, depending on 'outdated_after' configuration.
    pub ignore_verified: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Reverify snapshots after X days, never if 0. Ignored if 'ignore_verified' is false.
    pub outdated_after: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// when to schedule this job in calendar event notation
    pub schedule: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    /// on which backup namespace to run the verification recursively
    pub ns: Option<BackupNamespace>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    /// how deep the verify should go from the `ns` level downwards. Passing 0 verifies only the
    /// snapshots on the same level as the passed `ns`, or the datastore root if none.
    pub max_depth: Option<usize>,
}

impl VerificationJobConfig {
    pub fn acl_path(&self) -> Vec<&str> {
        match self.ns.as_ref() {
            Some(ns) => ns.acl_path(&self.store),
            None => vec!["datastore", &self.store],
        }
    }
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
#[derive(Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "kebab-case")]
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
        "group-filter": {
            schema: GROUP_FILTER_LIST_SCHEMA,
            optional: true,
        },
        ns: {
            type: BackupNamespace,
            optional: true,
        },
        "max-depth": {
            schema: crate::NS_MAX_DEPTH_SCHEMA,
            optional: true,
        },
    }
)]
#[derive(Serialize, Deserialize, Clone, Updater, PartialEq)]
#[serde(rename_all = "kebab-case")]
/// Tape Backup Job Setup
pub struct TapeBackupJobSetup {
    pub store: String,
    pub pool: String,
    pub drive: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eject_media: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub export_media_set: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_only: Option<bool>,
    /// Send job email notification to this user
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notify_user: Option<Userid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_filter: Option<Vec<GroupFilter>>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub ns: Option<BackupNamespace>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub max_depth: Option<usize>,
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
#[derive(Serialize, Deserialize, Clone, Updater, PartialEq)]
#[serde(rename_all = "kebab-case")]
/// Tape Backup Job
pub struct TapeBackupJobConfig {
    #[updater(skip)]
    pub id: String,
    #[serde(flatten)]
    pub setup: TapeBackupJobSetup,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
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
#[derive(Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "kebab-case")]
/// Status of Tape Backup Job
pub struct TapeBackupJobStatus {
    #[serde(flatten)]
    pub config: TapeBackupJobConfig,
    #[serde(flatten)]
    pub status: JobScheduleStatus,
    /// Next tape used (best guess)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_media_label: Option<String>,
}

#[derive(Clone, Debug)]
/// Filter for matching `BackupGroup`s, for use with `BackupGroup::filter`.
pub enum FilterType {
    /// BackupGroup type - either `vm`, `ct`, or `host`.
    BackupType(BackupType),
    /// Full identifier of BackupGroup, including type
    Group(String),
    /// A regular expression matched against the full identifier of the BackupGroup
    Regex(Regex),
}

impl PartialEq for FilterType {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::BackupType(a), Self::BackupType(b)) => a == b,
            (Self::Group(a), Self::Group(b)) => a == b,
            (Self::Regex(a), Self::Regex(b)) => a.as_str() == b.as_str(),
            _ => false,
        }
    }
}

impl std::str::FromStr for FilterType {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.split_once(':') {
            Some(("group", value)) => BACKUP_GROUP_SCHEMA.parse_simple_value(value).map(|_| FilterType::Group(value.to_string()))?,
            Some(("type", value)) => FilterType::BackupType(value.parse()?),
            Some(("regex", value)) => FilterType::Regex(Regex::new(value)?),
            Some((ty, _value)) => bail!("expected 'group', 'type' or 'regex' prefix, got '{}'", ty),
            None => bail!("input doesn't match expected format '<group:GROUP||type:<vm|ct|host>|regex:REGEX>'"),
        })
    }
}

// used for serializing below, caution!
impl std::fmt::Display for FilterType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FilterType::BackupType(backup_type) => write!(f, "type:{}", backup_type),
            FilterType::Group(backup_group) => write!(f, "group:{}", backup_group),
            FilterType::Regex(regex) => write!(f, "regex:{}", regex.as_str()),
        }
    }
}

#[derive(Clone, Debug)]
pub struct GroupFilter {
    pub is_exclude: bool,
    pub filter_type: FilterType,
}

impl PartialEq for GroupFilter {
    fn eq(&self, other: &Self) -> bool {
        self.filter_type == other.filter_type && self.is_exclude == other.is_exclude
    }
}

impl Eq for GroupFilter {}

impl std::str::FromStr for GroupFilter {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (is_exclude, type_str) = match s.split_once(':') {
            Some(("include", value)) => (false, value),
            Some(("exclude", value)) => (true, value),
            _ => (false, s),
        };

        Ok(GroupFilter {
            is_exclude,
            filter_type: type_str.parse()?,
        })
    }
}

// used for serializing below, caution!
impl std::fmt::Display for GroupFilter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_exclude {
            f.write_str("exclude:")?;
        }
        std::fmt::Display::fmt(&self.filter_type, f)
    }
}

proxmox_serde::forward_deserialize_to_from_str!(GroupFilter);
proxmox_serde::forward_serialize_to_display!(GroupFilter);

fn verify_group_filter(input: &str) -> Result<(), anyhow::Error> {
    GroupFilter::from_str(input).map(|_| ())
}

pub const GROUP_FILTER_SCHEMA: Schema = StringSchema::new(
    "Group filter based on group identifier ('group:GROUP'), group type ('type:<vm|ct|host>'), or regex ('regex:RE'). Can be inverted by prepending 'exclude:'.")
    .format(&ApiStringFormat::VerifyFn(verify_group_filter))
    .type_text("[<exclude:|include:>]<type:<vm|ct|host>|group:GROUP|regex:RE>")
    .schema();

pub const GROUP_FILTER_LIST_SCHEMA: Schema =
    ArraySchema::new("List of group filters.", &GROUP_FILTER_SCHEMA).schema();

pub const TRANSFER_LAST_SCHEMA: Schema =
    IntegerSchema::new("Limit transfer to last N snapshots (per group), skipping others")
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
        ns: {
            type: BackupNamespace,
            optional: true,
        },
        "owner": {
            type: Authid,
            optional: true,
        },
        remote: {
            schema: REMOTE_ID_SCHEMA,
            optional: true,
        },
        "remote-store": {
            schema: DATASTORE_SCHEMA,
        },
        "remote-ns": {
            type: BackupNamespace,
            optional: true,
        },
        "remove-vanished": {
            schema: REMOVE_VANISHED_BACKUPS_SCHEMA,
            optional: true,
        },
        "max-depth": {
            schema: NS_MAX_DEPTH_REDUCED_SCHEMA,
            optional: true,
        },
        comment: {
            optional: true,
            schema: SINGLE_LINE_COMMENT_SCHEMA,
        },
        limit: {
            type: RateLimitConfig,
        },
        schedule: {
            optional: true,
            schema: SYNC_SCHEDULE_SCHEMA,
        },
        "group-filter": {
            schema: GROUP_FILTER_LIST_SCHEMA,
            optional: true,
        },
        "transfer-last": {
            schema: TRANSFER_LAST_SCHEMA,
            optional: true,
        },
    }
)]
#[derive(Serialize, Deserialize, Clone, Updater, PartialEq)]
#[serde(rename_all = "kebab-case")]
/// Sync Job
pub struct SyncJobConfig {
    #[updater(skip)]
    pub id: String,
    pub store: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ns: Option<BackupNamespace>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<Authid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// None implies local sync.
    pub remote: Option<String>,
    pub remote_store: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_ns: Option<BackupNamespace>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remove_vanished: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_depth: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schedule: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_filter: Option<Vec<GroupFilter>>,
    #[serde(flatten)]
    pub limit: RateLimitConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transfer_last: Option<usize>,
}

impl SyncJobConfig {
    pub fn acl_path(&self) -> Vec<&str> {
        match self.ns.as_ref() {
            Some(ns) => ns.acl_path(&self.store),
            None => vec!["datastore", &self.store],
        }
    }
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
#[derive(Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "kebab-case")]
/// Status of Sync Job
pub struct SyncJobStatus {
    #[serde(flatten)]
    pub config: SyncJobConfig,
    #[serde(flatten)]
    pub status: JobScheduleStatus,
}

/// These are used separately without `ns`/`max-depth` sometimes in the API, specifically in the API
/// call to prune a specific group, where `max-depth` makes no sense.
#[api(
    properties: {
        "keep-last": {
            schema: crate::PRUNE_SCHEMA_KEEP_LAST,
            optional: true,
        },
        "keep-hourly": {
            schema: crate::PRUNE_SCHEMA_KEEP_HOURLY,
            optional: true,
        },
        "keep-daily": {
            schema: crate::PRUNE_SCHEMA_KEEP_DAILY,
            optional: true,
        },
        "keep-weekly": {
            schema: crate::PRUNE_SCHEMA_KEEP_WEEKLY,
            optional: true,
        },
        "keep-monthly": {
            schema: crate::PRUNE_SCHEMA_KEEP_MONTHLY,
            optional: true,
        },
        "keep-yearly": {
            schema: crate::PRUNE_SCHEMA_KEEP_YEARLY,
            optional: true,
        },
    }
)]
#[derive(Serialize, Deserialize, Default, Updater, Clone, PartialEq)]
#[serde(rename_all = "kebab-case")]
/// Common pruning options
pub struct KeepOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keep_last: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keep_hourly: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keep_daily: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keep_weekly: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keep_monthly: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keep_yearly: Option<u64>,
}

impl KeepOptions {
    pub fn keeps_something(&self) -> bool {
        self.keep_last.unwrap_or(0)
            + self.keep_hourly.unwrap_or(0)
            + self.keep_daily.unwrap_or(0)
            + self.keep_weekly.unwrap_or(0)
            + self.keep_monthly.unwrap_or(0)
            + self.keep_yearly.unwrap_or(0)
            > 0
    }
}

#[api(
    properties: {
        keep: {
            type: KeepOptions,
        },
        ns: {
            type: BackupNamespace,
            optional: true,
        },
        "max-depth": {
            schema: NS_MAX_DEPTH_REDUCED_SCHEMA,
            optional: true,
        },
    }
)]
#[derive(Serialize, Deserialize, Default, Updater, Clone, PartialEq)]
#[serde(rename_all = "kebab-case")]
/// Common pruning options
pub struct PruneJobOptions {
    #[serde(flatten)]
    pub keep: KeepOptions,

    /// The (optional) recursion depth
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_depth: Option<usize>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub ns: Option<BackupNamespace>,
}

impl PruneJobOptions {
    pub fn keeps_something(&self) -> bool {
        self.keep.keeps_something()
    }

    pub fn acl_path<'a>(&'a self, store: &'a str) -> Vec<&'a str> {
        match &self.ns {
            Some(ns) => ns.acl_path(store),
            None => vec!["datastore", store],
        }
    }
}

#[api(
    properties: {
        disable: {
            type: Boolean,
            optional: true,
            default: false,
        },
        id: {
            schema: JOB_ID_SCHEMA,
        },
        store: {
            schema: DATASTORE_SCHEMA,
        },
        schedule: {
            schema: PRUNE_SCHEDULE_SCHEMA,
        },
        comment: {
            optional: true,
            schema: SINGLE_LINE_COMMENT_SCHEMA,
        },
        options: {
            type: PruneJobOptions,
        },
    },
)]
#[derive(Deserialize, Serialize, Updater, Clone, PartialEq)]
#[serde(rename_all = "kebab-case")]
/// Prune configuration.
pub struct PruneJobConfig {
    /// unique ID to address this job
    #[updater(skip)]
    pub id: String,

    pub store: String,

    /// Disable this job.
    #[serde(default, skip_serializing_if = "is_false")]
    #[updater(serde(skip_serializing_if = "Option::is_none"))]
    pub disable: bool,

    pub schedule: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,

    #[serde(flatten)]
    pub options: PruneJobOptions,
}

impl PruneJobConfig {
    pub fn acl_path(&self) -> Vec<&str> {
        self.options.acl_path(&self.store)
    }
}

fn is_false(b: &bool) -> bool {
    !b
}

#[api(
    properties: {
        config: {
            type: PruneJobConfig,
        },
        status: {
            type: JobScheduleStatus,
        },
    },
)]
#[derive(Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "kebab-case")]
/// Status of prune job
pub struct PruneJobStatus {
    #[serde(flatten)]
    pub config: PruneJobConfig,
    #[serde(flatten)]
    pub status: JobScheduleStatus,
}
