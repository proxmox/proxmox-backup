use std::collections::{HashMap, HashSet};

use ::serde::{Deserialize, Serialize};
use anyhow::{bail, Error};
use lazy_static::lazy_static;

use proxmox_schema::api;

#[api()]
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
/// SMART status
pub enum SmartStatus {
    /// Smart tests passed - everything is OK
    Passed,
    /// Smart tests failed - disk has problems
    Failed,
    /// Unknown status
    Unknown,
}

#[api()]
#[derive(Debug, Serialize, Deserialize)]
/// SMART Attribute
pub struct SmartAttribute {
    /// Attribute name
    name: String,
    // FIXME: remove value with next major release (PBS 3.0)
    /// duplicate of raw - kept for API stability
    value: String,
    /// Attribute raw value
    raw: String,
    // the rest of the values is available for ATA type
    /// ATA Attribute ID
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<u64>,
    /// ATA Flags
    #[serde(skip_serializing_if = "Option::is_none")]
    flags: Option<String>,
    /// ATA normalized value (0..100)
    #[serde(skip_serializing_if = "Option::is_none")]
    normalized: Option<f64>,
    /// ATA worst
    #[serde(skip_serializing_if = "Option::is_none")]
    worst: Option<f64>,
    /// ATA threshold
    #[serde(skip_serializing_if = "Option::is_none")]
    threshold: Option<f64>,
}

#[api(
    properties: {
        status: {
            type: SmartStatus,
        },
        wearout: {
            description: "Wearout level.",
            type: f64,
            optional: true,
        },
        attributes: {
            description: "SMART attributes.",
            type: Array,
            items: {
                type: SmartAttribute,
            },
        },
    },
)]
#[derive(Debug, Serialize, Deserialize)]
/// Data from smartctl
pub struct SmartData {
    pub status: SmartStatus,
    pub wearout: Option<f64>,
    pub attributes: Vec<SmartAttribute>,
}

/// Read smartctl data for a disk (/dev/XXX).
pub fn get_smart_data(disk: &super::Disk, health_only: bool) -> Result<SmartData, Error> {
    const SMARTCTL_BIN_PATH: &str = "smartctl";

    let mut command = std::process::Command::new(SMARTCTL_BIN_PATH);
    command.arg("-H");
    if !health_only {
        command.args(["-A", "-j"]);
    }

    let disk_path = match disk.device_path() {
        Some(path) => path,
        None => bail!("disk {:?} has no node in /dev", disk.syspath()),
    };
    command.arg(disk_path);

    let output = proxmox_sys::command::run_command(
        command,
        Some(
            |exitcode| (exitcode & 0b0011) == 0, // only bits 0-1 are fatal errors
        ),
    )?;

    let output: serde_json::Value = output.parse()?;

    let mut wearout = None;

    let mut attributes = Vec::new();
    let mut wearout_candidates = HashMap::new();

    // ATA devices
    if let Some(list) = output["ata_smart_attributes"]["table"].as_array() {
        for item in list {
            let id = match item["id"].as_u64() {
                Some(id) => id,
                None => continue, // skip attributes without id
            };

            let name = match item["name"].as_str() {
                Some(name) => name.to_string(),
                None => continue, // skip attributes without name
            };

            let raw_value = match item["raw"]["string"].as_str() {
                Some(value) => value.to_string(),
                None => continue, // skip attributes without raw value
            };

            let flags = match item["flags"]["string"].as_str() {
                Some(flags) => flags.to_string(),
                None => continue, // skip attributes without flags
            };

            let normalized = match item["value"].as_f64() {
                Some(v) => v,
                None => continue, // skip attributes without normalize value
            };

            let worst = match item["worst"].as_f64() {
                Some(v) => v,
                None => continue, // skip attributes without worst entry
            };

            let threshold = match item["thresh"].as_f64() {
                Some(v) => v,
                None => continue, // skip attributes without threshold entry
            };

            if WEAROUT_FIELD_NAMES.contains(&name as &str) {
                wearout_candidates.insert(name.clone(), normalized);
            }

            attributes.push(SmartAttribute {
                name,
                value: raw_value.clone(),
                raw: raw_value,
                id: Some(id),
                flags: Some(flags),
                normalized: Some(normalized),
                worst: Some(worst),
                threshold: Some(threshold),
            });
        }
    }

    if !wearout_candidates.is_empty() {
        for field in WEAROUT_FIELD_ORDER {
            if let Some(value) = wearout_candidates.get(field as &str) {
                wearout = Some(*value);
                break;
            }
        }
    }

    // NVME devices
    if let Some(list) = output["nvme_smart_health_information_log"].as_object() {
        for (name, value) in list {
            if name == "percentage_used" {
                // extract wearout from nvme text, allow for decimal values
                if let Some(v) = value.as_f64() {
                    if v <= 100.0 {
                        wearout = Some(100.0 - v);
                    }
                }
            }
            if let Some(value) = value.as_f64() {
                attributes.push(SmartAttribute {
                    name: name.to_string(),
                    value: value.to_string(),
                    raw: value.to_string(),
                    id: None,
                    flags: None,
                    normalized: None,
                    worst: None,
                    threshold: None,
                });
            }
        }
    }

    let status = match output["smart_status"]["passed"].as_bool() {
        None => SmartStatus::Unknown,
        Some(true) => SmartStatus::Passed,
        Some(false) => SmartStatus::Failed,
    };

    Ok(SmartData {
        status,
        wearout,
        attributes,
    })
}

static WEAROUT_FIELD_ORDER: &[&str] = &[
    "Media_Wearout_Indicator",
    "SSD_Life_Left",
    "Wear_Leveling_Count",
    "Perc_Write/Erase_Ct_BC",
    "Perc_Rated_Life_Remain",
    "Remaining_Lifetime_Perc",
    "Percent_Lifetime_Remain",
    "Lifetime_Left",
    "PCT_Life_Remaining",
    "Lifetime_Remaining",
    "Percent_Life_Remaining",
    "Percent_Lifetime_Used",
    "Perc_Rated_Life_Used",
];

lazy_static! {
    static ref WEAROUT_FIELD_NAMES: HashSet<&'static str> =
        WEAROUT_FIELD_ORDER.iter().cloned().collect();
}
