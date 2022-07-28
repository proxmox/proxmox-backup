use serde::{Deserialize, Serialize};

use pbs_api_types::SINGLE_LINE_COMMENT_FORMAT;
use proxmox_schema::*;

pub const SYSTEMD_SECTION_NAME_SCHEMA: Schema = StringSchema::new("Section name")
    .format(&ApiStringFormat::Enum(&[
        EnumEntry::new("Unit", "Unit"),
        EnumEntry::new("Timer", "Timer"),
        EnumEntry::new("Install", "Install"),
        EnumEntry::new("Mount", "Mount"),
        EnumEntry::new("Service", "Service"),
    ]))
    .schema();

pub const SYSTEMD_STRING_SCHEMA: Schema = StringSchema::new("Systemd configuration value.")
    .format(&SINGLE_LINE_COMMENT_FORMAT)
    .schema();

pub const SYSTEMD_STRING_ARRAY_SCHEMA: Schema =
    ArraySchema::new("Array of Strings", &SYSTEMD_STRING_SCHEMA).schema();

pub const SYSTEMD_TIMESPAN_ARRAY_SCHEMA: Schema =
    ArraySchema::new("Array of time spans", &SYSTEMD_TIMESPAN_SCHEMA).schema();

pub const SYSTEMD_CALENDAR_EVENT_ARRAY_SCHEMA: Schema =
    ArraySchema::new("Array of calendar events", &SYSTEMD_CALENDAR_EVENT_SCHEMA).schema();

#[api(
    properties: {
        "OnCalendar": {
            schema: SYSTEMD_CALENDAR_EVENT_ARRAY_SCHEMA,
            optional: true,
        },
        "OnActiveSec": {
            schema: SYSTEMD_TIMESPAN_ARRAY_SCHEMA,
            optional: true,
        },
        "OnBootSec": {
            schema: SYSTEMD_TIMESPAN_ARRAY_SCHEMA,
            optional: true,
        },
        "OnStartupSec": {
            schema: SYSTEMD_TIMESPAN_ARRAY_SCHEMA,
            optional: true,
        },
        "OnUnitActiveSec": {
            schema: SYSTEMD_TIMESPAN_ARRAY_SCHEMA,
            optional: true,
        },
        "OnUnitInactiveSec": {
            schema: SYSTEMD_TIMESPAN_ARRAY_SCHEMA,
            optional: true,
        },
        "RandomizedDelaySec": {
            schema: SYSTEMD_TIMESPAN_SCHEMA,
            optional: true,
        },
        "AccuracySec": {
            schema: SYSTEMD_TIMESPAN_SCHEMA,
            optional: true,
        },
    },
)]
#[derive(Serialize, Deserialize, Default)]
#[allow(non_snake_case)]
/// Systemd Timer Section
pub struct SystemdTimerSection {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub OnCalendar: Option<Vec<String>>,
    ///  If true, the time when the service unit was last triggered is stored on disk.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub Persistent: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub OnActiveSec: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub OnBootSec: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub OnStartupSec: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub OnUnitActiveSec: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub OnUnitInactiveSec: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub RandomizedDelaySec: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub AccuracySec: Option<String>,

    /// Trigger when system clock jumps.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub OnClockChange: Option<bool>,

    /// Trigger when time zone changes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub OnTimezomeChange: Option<bool>,

    /// The unit to activate when this timer elapses.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub Unit: Option<String>,

    /// If true, an elapsing timer will cause the system to resume from suspend.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub WakeSystem: Option<bool>,

    /// If true, an elapsed timer will stay loaded, and its state remains queryable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub RemainAfterElapse: Option<bool>,
}

#[api(
    properties: {
        "Type": {
            type: ServiceStartup,
            optional: true,
        },
        "ExecStart": {
            schema: SYSTEMD_STRING_ARRAY_SCHEMA,
            optional: true,
        },
    }
)]
#[derive(Serialize, Deserialize, Default)]
#[allow(non_snake_case)]
/// Systemd Service Section
pub struct SystemdServiceSection {
    /// The process start-up type for this service unit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub Type: Option<ServiceStartup>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ExecStart: Option<Vec<String>>,
}

#[api()]
#[derive(Serialize, Deserialize, Default)]
#[allow(non_snake_case)]
/// Systemd Unit Section
pub struct SystemdUnitSection {
    /// A human readable name for the unit.
    pub Description: String,
    /// Check whether the system has AC power.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ConditionACPower: Option<bool>,
}

#[api(
    properties: {
        "Alias": {
            schema: SYSTEMD_STRING_ARRAY_SCHEMA,
            optional: true,
        },
        "Also": {
            schema: SYSTEMD_STRING_ARRAY_SCHEMA,
            optional: true,
        },
        "DefaultInstance":  {
            optional: true,
        },
        "WantedBy": {
            schema: SYSTEMD_STRING_ARRAY_SCHEMA,
            optional: true,
        },
        "RequiredBy": {
            schema: SYSTEMD_STRING_ARRAY_SCHEMA,
            optional: true,
        },
    },
)]
#[derive(Serialize, Deserialize, Default)]
#[allow(non_snake_case)]
/// Systemd Install Section
pub struct SystemdInstallSection {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub Alias: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub Also: Option<Vec<String>>,
    /// DefaultInstance for template unit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub DefaultInstance: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub WantedBy: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub RequiredBy: Option<Vec<String>>,
}

#[api(
    properties: {
        "TimeoutSec": {
            schema: SYSTEMD_TIMESPAN_ARRAY_SCHEMA,
            optional: true,
        },
    }
)]
#[derive(Serialize, Deserialize, Default)]
#[allow(non_snake_case)]
/// Systemd Service Section
pub struct SystemdMountSection {
    /// absolute path of a device node, file or other resource to mount
    pub What: String,
    /// absolute path of a file or directory for the mount point
    pub Where: String,
    /// Takes a string for the file system type. See mount(8) for details.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub Type: Option<String>,
    /// Mount options to use when mounting. This takes a comma-separated list of options.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub Options: Option<String>,
    /// If true, parsing of the options specified in Options= is relaxed, and unknown mount options are tolerated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub SloppyOptions: Option<bool>,
    /// Use lazy unmount
    #[serde(skip_serializing_if = "Option::is_none")]
    pub LazyUnmount: Option<bool>,
    /// Use forces unmount
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ForceUnmount: Option<bool>,
    /// Directories of mount points (and any parent directories) are
    /// automatically created if needed. Takes an access mode in octal
    /// notation. Defaults to 0755.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub DirectoryMode: Option<String>,
    /// Configures the time to wait for the mount command to finish.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub TimeoutSec: Option<String>,
}

#[api()]
#[derive(Debug, Copy, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
/// Node Power command type.
pub enum ServiceStartup {
    /// Simple fork
    Simple,
    /// Like fork, but wait until exec succeeds
    Exec,
    /// Fork daemon
    Forking,
    /// Like 'simple', but consider the unit up after the process exits.
    Oneshot,
    /// Like 'simple', but use DBUS to synchronize startup.
    Dbus,
    /// Like 'simple', but use sd_notify to synchronize startup.
    Notify,
}

pub const SYSTEMD_TIMESPAN_SCHEMA: Schema = StringSchema::new("systemd time span")
    .format(&ApiStringFormat::VerifyFn(proxmox_time::verify_time_span))
    .schema();

pub const SYSTEMD_CALENDAR_EVENT_SCHEMA: Schema = StringSchema::new("systemd calendar event")
    .format(&ApiStringFormat::VerifyFn(
        proxmox_time::verify_calendar_event,
    ))
    .schema();
