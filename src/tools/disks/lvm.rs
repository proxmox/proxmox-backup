use std::collections::{HashSet, HashMap};

use anyhow::{format_err, Error};
use serde_json::Value;
use lazy_static::lazy_static;

lazy_static!{
    static ref LVM_UUIDS: HashSet<&'static str> = {
        let mut set = HashSet::new();
	set.insert("e6d6d379-f507-44c2-a23c-238f2a3df928");
        set
    };
}

/// Get list of devices used by LVM (pvs).
pub fn get_lvm_devices(
    partition_type_map: &HashMap<String, Vec<String>>,
) -> Result<HashSet<String>, Error> {

    const PVS_BIN_PATH: &str = "/sbin/pvs";

    let mut command = std::process::Command::new(PVS_BIN_PATH);
    command.args(&["--reportformat", "json", "--noheadings", "--readonly", "-o", "pv_name"]);

    let output = command.output()
        .map_err(|err| format_err!("failed to execute '{}' - {}", PVS_BIN_PATH, err))?;

    let output = crate::tools::command_output(output, None)
        .map_err(|err| format_err!("pvs command failed: {}", err))?;

    let mut device_set: HashSet<String> = HashSet::new();

    for device_list in partition_type_map.iter()
        .filter_map(|(uuid, list)| if LVM_UUIDS.contains(uuid.as_str()) { Some(list) } else { None })
    {
        for device in device_list {
            device_set.insert(device.clone());
        }
    }

    let output: Value = output.parse()?;

    match output["report"][0]["pv"].as_array() {
        Some(list) => {
            for info in list {
                if let Some(pv_name) = info["pv_name"].as_str() {
                    device_set.insert(pv_name.to_string());
                }
            }
        }
        None => return Ok(device_set),
    }

    Ok(device_set)
}
