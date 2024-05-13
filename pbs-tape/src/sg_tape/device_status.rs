use std::os::fd::AsRawFd;

use anyhow::{bail, format_err, Error};

use pbs_api_types::DeviceActivity;
use proxmox_io::ReadExt;

use super::LpParameterHeader;
use crate::sgutils2::SgRaw;

/// SCSI command to query volume statistics
///
/// CDB: LOG SENSE / LP11h DT Device Activity
///
/// Only returns the Device Activity result from the VHF data
pub fn read_device_activity<F: AsRawFd>(file: &mut F) -> Result<DeviceActivity, Error> {
    let data = sg_read_dt_device_status(file)?;

    decode_dt_device_status(&data)
        .map_err(|err| format_err!("decode dt device status failed - {}", err))
}

#[allow(clippy::vec_init_then_push)]
fn sg_read_dt_device_status<F: AsRawFd>(file: &mut F) -> Result<Vec<u8>, Error> {
    let alloc_len: u16 = 8192;
    let mut sg_raw = SgRaw::new(file, alloc_len as usize)?;

    let mut cmd = Vec::new();
    cmd.push(0x4D); // LOG SENSE
    cmd.push(0);
    cmd.push((1 << 6) | 0x11); // DT Device Status log page
    cmd.push(0); // Subpage 0
    cmd.push(0);
    cmd.push(0);
    cmd.push(0);
    cmd.extend(alloc_len.to_be_bytes()); // alloc len
    cmd.push(0u8); // control byte

    sg_raw.set_timeout(1); // use short timeout
    sg_raw
        .do_command(&cmd)
        .map_err(|err| format_err!("read tape dt device status failed - {}", err))
        .map(|v| v.to_vec())
}

fn decode_dt_device_status(data: &[u8]) -> Result<DeviceActivity, Error> {
    if !((data[0] & 0x7f) == 0x11 && data[1] == 0) {
        bail!("invalid response");
    }

    let mut reader = &data[2..];

    let page_len: u16 = unsafe { reader.read_be_value()? };

    let page_len = page_len as usize;

    if (page_len + 4) > data.len() {
        bail!("invalid page length");
    } else {
        // Note: Quantum hh7 returns the allocation_length instead of real data_len
        reader = &data[4..page_len + 4];
    }

    let mut page_valid = false;

    let mut activity = DeviceActivity::Other;

    loop {
        if reader.is_empty() {
            break;
        }
        let head: LpParameterHeader = unsafe { reader.read_be_value()? };

        match head.parameter_code {
            0x0000 => {
                let vhf_descriptor = reader.read_exact_allocated(head.parameter_len as usize)?;

                if vhf_descriptor.len() != 4 {
                    bail!("invalid VHF data descriptor");
                }

                activity = vhf_descriptor[2].try_into()?;

                if vhf_descriptor[0] & 0x01 == 1 {
                    page_valid = true;
                }
            }
            _ => {
                reader.read_exact_allocated(head.parameter_len as usize)?;
            }
        }
    }

    if !page_valid {
        bail!("missing page-valid parameter");
    }

    Ok(activity)
}
