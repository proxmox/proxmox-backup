use anyhow::{bail, Error};
use ::serde::{Deserialize, Serialize};

use proxmox::api::api;

#[api()]
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all="lowercase")]
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
    /// Attribute raw value
    value: String,
    // the rest of the values is available for ATA type
    /// ATA Attribute ID
    #[serde(skip_serializing_if="Option::is_none")]
    id: Option<u64>,
    /// ATA Flags
    #[serde(skip_serializing_if="Option::is_none")]
    flags: Option<String>,
    /// ATA normalized value (0..100)
    #[serde(skip_serializing_if="Option::is_none")]
    normalized: Option<f64>,
    /// ATA worst
    #[serde(skip_serializing_if="Option::is_none")]
    worst: Option<f64>,
    /// ATA threshold
    #[serde(skip_serializing_if="Option::is_none")]
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
pub fn get_smart_data(
    disk: &super::Disk,
    health_only: bool,
) -> Result<SmartData, Error> {

    const SMARTCTL_BIN_PATH: &str = "/usr/sbin/smartctl";

    let mut command = std::process::Command::new(SMARTCTL_BIN_PATH);
    command.arg("-H");
    if !health_only { command.args(&["-A", "-j"]); }

    let disk_path = match disk.device_path() {
        Some(path) => path,
        None => bail!("disk {:?} has no node in /dev", disk.syspath()),
    };
    command.arg(disk_path);

    let output = crate::tools::run_command(command, None)?;

    let output: serde_json::Value = output.parse()?;

    let mut wearout = None;

    let mut attributes = Vec::new();

    // ATA devices
    if let Some(list) = output["ata_smart_attributes"]["table"].as_array() {
        let wearout_id = lookup_vendor_wearout_id(disk);
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

            if id == wearout_id {
                wearout = Some(normalized);
            }

            attributes.push(SmartAttribute {
                name,
                value: raw_value,
                id: Some(id),
                flags: Some(flags),
                normalized: Some(normalized),
                worst: Some(worst),
                threshold: Some(threshold),
            });
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


    Ok(SmartData { status, wearout, attributes })
}

fn lookup_vendor_wearout_id(disk: &super::Disk) -> u64 {

    static VENDOR_MAP: &[(&str, u64)] = &[
        ("kingston", 231),
        ("samsung", 177),
        ("intel", 233),
        ("sandisk", 233),
        ("crucial", 202),
    ];

    let result = 233; // default
    let model = match disk.model() {
        Some(model) => model.to_string_lossy().to_lowercase(),
        None => return result,
    };

    for (vendor, attr_id) in VENDOR_MAP {
        if model.contains(vendor) {
            return *attr_id;
        }
    }

    result
}
