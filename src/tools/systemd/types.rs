use serde::{Serialize, Deserialize};

use proxmox::api::{ api, schema::* };
use crate::api2::types::SINGLE_LINE_COMMENT_FORMAT;

pub const SYSTEMD_SECTION_NAME_SCHEMA: Schema = StringSchema::new(
    "Section name")
    .format(&ApiStringFormat::Enum(&[
        EnumEntry::new("Unit", "Unit"),
        EnumEntry::new("Timer", "Timer"),
        EnumEntry::new("Install", "Install"),
        EnumEntry::new("Service", "Service")]))
    .schema();

pub const SYSTEMD_STRING_SCHEMA: Schema =
    StringSchema::new("Systemd configuration value.")
    .format(&SINGLE_LINE_COMMENT_FORMAT)
    .schema();

pub const SYSTEMD_STRING_ARRAY_SCHEMA: Schema = ArraySchema::new(
    "Array of Strings", &SYSTEMD_STRING_SCHEMA)
    .schema();

pub const SYSTEMD_TIMESPAN_ARRAY_SCHEMA: Schema = ArraySchema::new(
    "Array of time spans", &SYSTEMD_TIMESPAN_SCHEMA)
    .schema();

pub const SYSTEMD_CALENDAR_EVENT_ARRAY_SCHEMA: Schema = ArraySchema::new(
    "Array of calendar events", &SYSTEMD_CALENDAR_EVENT_SCHEMA)
    .schema();

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
    #[serde(skip_serializing_if="Option::is_none")]
    pub OnCalendar: Option<Vec<String>>,
    ///  If true, the time when the service unit was last triggered is stored on disk.
    #[serde(skip_serializing_if="Option::is_none")]
    pub Persistent: Option<bool>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub OnActiveSec: Option<Vec<String>>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub OnBootSec: Option<Vec<String>>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub OnStartupSec: Option<Vec<String>>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub OnUnitActiveSec: Option<Vec<String>>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub OnUnitInactiveSec: Option<Vec<String>>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub RandomizedDelaySec: Option<String>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub AccuracySec: Option<String>,

    /// Trigger when system clock jumps.
    #[serde(skip_serializing_if="Option::is_none")]
    pub OnClockChange: Option<bool>,

    /// Trigger when time zone changes.
    #[serde(skip_serializing_if="Option::is_none")]
    pub OnTimezomeChange: Option<bool>,

    /// The unit to activate when this timer elapses.
    #[serde(skip_serializing_if="Option::is_none")]
    pub Unit: Option<String>,

    /// If true, an elapsing timer will cause the system to resume from suspend.
    #[serde(skip_serializing_if="Option::is_none")]
    pub WakeSystem: Option<bool>,

    /// If true, an elapsed timer will stay loaded, and its state remains queryable.
    #[serde(skip_serializing_if="Option::is_none")]
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
    #[serde(skip_serializing_if="Option::is_none")]
    pub Type: Option<ServiceStartup>,
    #[serde(skip_serializing_if="Option::is_none")]
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
    #[serde(skip_serializing_if="Option::is_none")]
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
    #[serde(skip_serializing_if="Option::is_none")]
    pub Alias: Option<Vec<String>>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub Also: Option<Vec<String>>,
    /// DefaultInstance for template unit.
    #[serde(skip_serializing_if="Option::is_none")]
    pub DefaultInstance: Option<String>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub WantedBy: Option<Vec<String>>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub RequiredBy: Option<Vec<String>>,
}

#[api()]
#[derive(Debug, Copy, Clone, PartialEq, Serialize, Deserialize)]
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

pub const SYSTEMD_TIMESPAN_SCHEMA: Schema = StringSchema::new(
    "systemd time span")
    .format(&ApiStringFormat::VerifyFn(super::time::verify_time_span))
    .schema();

pub const SYSTEMD_CALENDAR_EVENT_SCHEMA: Schema = StringSchema::new(
    "systemd time span")
    .format(&ApiStringFormat::VerifyFn(super::time::verify_calendar_event))
    .schema();
