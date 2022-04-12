use serde::{Deserialize, Serialize};

use proxmox_schema::{api, ApiStringFormat, const_regex, Schema, StringSchema};

const_regex!{
    pub MAINTENANCE_MESSAGE_REGEX = r"^[[:^cntrl:]]*$";
}

pub const MAINTENANCE_MESSAGE_FORMAT: ApiStringFormat =
    ApiStringFormat::Pattern(&MAINTENANCE_MESSAGE_REGEX);


pub const MAINTENANCE_MESSAGE_SCHEMA: Schema =
    StringSchema::new("Message describing the reason for the maintenance.")
        .format(&MAINTENANCE_MESSAGE_FORMAT)
        .max_length(64)
        .schema();

#[derive(Clone, Copy, Debug)]
/// Operation requirements, used when checking for maintenance mode.
pub enum Operation {
    Read,
    Write,
}

#[api]
#[derive(Deserialize, Serialize)]
#[serde(rename_all="kebab-case")]
/// Maintenance type.
pub enum MaintenanceType {
    /// Only read operations are allowed on the datastore.
    ReadOnly,
    /// Neither read nor write operations are allowed on the datastore.
    Offline,
}

#[api(
    properties: {
        type: {
            type: MaintenanceType,
        },
        message: {
            optional: true,
            schema: MAINTENANCE_MESSAGE_SCHEMA,
        }
    },
    default_key: "type",
)]
#[derive(Deserialize, Serialize)]
/// Maintenance mode
pub struct MaintenanceMode {
    /// Type of maintenance ("read-only" or "offline").
    #[serde(rename = "type")]
    ty: MaintenanceType,

    /// Reason for maintenance.
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}
