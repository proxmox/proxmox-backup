use std::collections::{HashSet, HashMap};
use std::os::unix::fs::MetadataExt;

use anyhow::{Error};
use serde_json::Value;
use lazy_static::lazy_static;

lazy_static!{
    static ref LVM_UUIDS: HashSet<&'static str> = {
        let mut set = HashSet::new();
	set.insert("e6d6d379-f507-44c2-a23c-238f2a3df928");
        set
    };
}

/// Get set of devices used by LVM (pvs).
///
/// The set is indexed by using the unix raw device number (dev_t is u64)
pub fn get_lvm_devices(
    partition_type_map: &HashMap<String, Vec<String>>,
) -> Result<HashSet<u64>, Error> {

    const PVS_BIN_PATH: &str = "/sbin/pvs";

    let mut command = std::process::Command::new(PVS_BIN_PATH);
    command.args(&["--reportformat", "json", "--noheadings", "--readonly", "-o", "pv_name"]);

    let output = crate::tools::run_command(command, None)?;

    let mut device_set: HashSet<u64> = HashSet::new();

    for device_list in partition_type_map.iter()
        .filter_map(|(uuid, list)| if LVM_UUIDS.contains(uuid.as_str()) { Some(list) } else { None })
    {
        for device in device_list {
            let meta = std::fs::metadata(device)?;
            device_set.insert(meta.rdev());
        }
    }

    let output: Value = output.parse()?;

    match output["report"][0]["pv"].as_array() {
        Some(list) => {
            for info in list {
                if let Some(pv_name) = info["pv_name"].as_str() {
                    let meta = std::fs::metadata(pv_name)?;
                    device_set.insert(meta.rdev());
                }
            }
        }
        None => return Ok(device_set),
    }

    Ok(device_set)
}
