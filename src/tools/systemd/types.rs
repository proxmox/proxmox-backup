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

#[api(
    properties: {
        "OnCalendar": {
            type: Array,
            optional: true,
            items: {
                description: "Calendar event expression.",
                type: String,
            },
        },
    },
)]
#[derive(Serialize, Deserialize, Default)]
#[allow(non_snake_case)]
/// Systemd Timer Section
pub struct SystemdTimerSection {
    /// Calender event list.
    #[serde(skip_serializing_if="Option::is_none")]
    pub OnCalendar: Option<Vec<String>>,
    ///  If true, the time when the service unit was last triggered is stored on disk.
    #[serde(skip_serializing_if="Option::is_none")]
    pub Persistent: Option<bool>,
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
