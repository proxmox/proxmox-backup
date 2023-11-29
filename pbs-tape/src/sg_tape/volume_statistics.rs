use std::io::Read;
use std::os::unix::io::AsRawFd;

use anyhow::{bail, format_err, Error};
use endian_trait::Endian;

use proxmox_io::ReadExt;

use pbs_api_types::Lp17VolumeStatistics;

use crate::sgutils2::SgRaw;

/// SCSI command to query volume statistics
///
/// CDB: LOG SENSE / LP17h Volume Statistics
///
/// The Volume Statistics log page is included in Ultrium 5 and later
/// drives.
pub fn read_volume_statistics<F: AsRawFd>(file: &mut F) -> Result<Lp17VolumeStatistics, Error> {
    let data = sg_read_volume_statistics(file)?;

    decode_volume_statistics(&data)
}

#[allow(clippy::vec_init_then_push)]
fn sg_read_volume_statistics<F: AsRawFd>(file: &mut F) -> Result<Vec<u8>, Error> {
    let alloc_len: u16 = 8192;
    let mut sg_raw = SgRaw::new(file, alloc_len as usize)?;

    let mut cmd = Vec::new();
    cmd.push(0x4D); // LOG SENSE
    cmd.push(0);
    cmd.push((1 << 6) | 0x17); // Volume Statistics log page
    cmd.push(0); // Subpage 0
    cmd.push(0);
    cmd.push(0);
    cmd.push(0);
    cmd.extend(alloc_len.to_be_bytes()); // alloc len
    cmd.push(0u8); // control byte

    sg_raw
        .do_command(&cmd)
        .map_err(|err| format_err!("read tape volume statistics failed - {}", err))
        .map(|v| v.to_vec())
}

#[repr(C, packed)]
#[derive(Endian)]
struct LpParameterHeader {
    parameter_code: u16,
    control: u8,
    parameter_len: u8,
}

fn decode_volume_statistics(data: &[u8]) -> Result<Lp17VolumeStatistics, Error> {
    let read_be_counter = |reader: &mut &[u8], len: u8| {
        let len = len as usize;
        if len == 0 || len > 8 {
            bail!("invalid counter size '{}'", len);
        }
        let mut buffer = [0u8; 8];
        reader.read_exact(&mut buffer[..len])?;

        let value = buffer
            .iter()
            .take(len)
            .fold(0, |value, curr| (value << 8) | *curr as u64);

        Ok(value)
    };

    proxmox_lang::try_block!({
        if !((data[0] & 0x7f) == 0x17 && data[1] == 0) {
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

        let mut stat = Lp17VolumeStatistics::default();
        let mut page_valid = false;

        loop {
            if reader.is_empty() {
                break;
            }
            let head: LpParameterHeader = unsafe { reader.read_be_value()? };

            match head.parameter_code {
                0x0000 => {
                    let value: u64 = read_be_counter(&mut reader, head.parameter_len)?;
                    if value == 0 {
                        bail!("page-valid flag not set");
                    }
                    page_valid = true;
                }
                0x0001 => {
                    stat.volume_mounts = read_be_counter(&mut reader, head.parameter_len)?;
                }
                0x0002 => {
                    stat.volume_datasets_written =
                        read_be_counter(&mut reader, head.parameter_len)?;
                }
                0x0003 => {
                    stat.volume_recovered_write_data_errors =
                        read_be_counter(&mut reader, head.parameter_len)?;
                }
                0x0004 => {
                    stat.volume_unrecovered_write_data_errors =
                        read_be_counter(&mut reader, head.parameter_len)?;
                }
                0x0005 => {
                    stat.volume_write_servo_errors =
                        read_be_counter(&mut reader, head.parameter_len)?;
                }
                0x0006 => {
                    stat.volume_unrecovered_write_servo_errors =
                        read_be_counter(&mut reader, head.parameter_len)?;
                }
                0x0007 => {
                    stat.volume_datasets_read = read_be_counter(&mut reader, head.parameter_len)?;
                }
                0x0008 => {
                    stat.volume_recovered_read_errors =
                        read_be_counter(&mut reader, head.parameter_len)?;
                }
                0x0009 => {
                    stat.volume_unrecovered_read_errors =
                        read_be_counter(&mut reader, head.parameter_len)?;
                }
                0x000C => {
                    stat.last_mount_unrecovered_write_errors =
                        read_be_counter(&mut reader, head.parameter_len)?;
                }
                0x000D => {
                    stat.last_mount_unrecovered_read_errors =
                        read_be_counter(&mut reader, head.parameter_len)?;
                }
                0x000E => {
                    stat.last_mount_bytes_written =
                        read_be_counter(&mut reader, head.parameter_len)? * 1_000_000;
                }
                0x000F => {
                    stat.last_mount_bytes_read =
                        read_be_counter(&mut reader, head.parameter_len)? * 1_000_000;
                }
                0x0010 => {
                    stat.lifetime_bytes_written =
                        read_be_counter(&mut reader, head.parameter_len)? * 1_000_000;
                }
                0x0011 => {
                    stat.lifetime_bytes_read =
                        read_be_counter(&mut reader, head.parameter_len)? * 1_000_000;
                }
                0x0012 => {
                    stat.last_load_write_compression_ratio =
                        read_be_counter(&mut reader, head.parameter_len)?;
                }
                0x0013 => {
                    stat.last_load_read_compression_ratio =
                        read_be_counter(&mut reader, head.parameter_len)?;
                }
                0x0014 => {
                    stat.medium_mount_time = read_be_counter(&mut reader, head.parameter_len)?;
                }
                0x0015 => {
                    stat.medium_ready_time = read_be_counter(&mut reader, head.parameter_len)?;
                }
                0x0016 => {
                    stat.total_native_capacity =
                        read_be_counter(&mut reader, head.parameter_len)? * 1_000_000;
                }
                0x0017 => {
                    stat.total_used_native_capacity =
                        read_be_counter(&mut reader, head.parameter_len)? * 1_000_000;
                }
                0x0040 => {
                    let data = reader.read_exact_allocated(head.parameter_len as usize)?;
                    stat.serial = String::from_utf8_lossy(&data).to_string();
                }
                0x0080 => {
                    let value = read_be_counter(&mut reader, head.parameter_len)?;
                    if value == 1 {
                        stat.write_protect = true;
                    }
                }
                0x0081 => {
                    let value = read_be_counter(&mut reader, head.parameter_len)?;
                    if value == 1 {
                        stat.worm = true;
                    }
                }
                0x0101 => {
                    stat.beginning_of_medium_passes =
                        read_be_counter(&mut reader, head.parameter_len)?;
                }
                0x0102 => {
                    stat.middle_of_tape_passes = read_be_counter(&mut reader, head.parameter_len)?;
                }
                _ => {
                    reader.read_exact_allocated(head.parameter_len as usize)?;
                }
            }
        }

        if !page_valid {
            bail!("missing page-valid parameter");
        }

        Ok(stat)
    })
    .map_err(|err| format_err!("decode volume statistics failed - {}", err))
}
