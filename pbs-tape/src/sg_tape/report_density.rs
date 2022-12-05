use anyhow::{bail, format_err, Error};
use endian_trait::Endian;
use std::io::Read;
use std::os::unix::io::AsRawFd;

use proxmox_io::ReadExt;

use crate::sgutils2::SgRaw;

#[repr(C, packed)]
#[derive(Endian)]
struct DesnityDescriptorBlock {
    primary_density_code: u8,
    secondary_density_code: u8,
    flags2: u8,
    reserved: [u8; 2],
    bits_per_mm: [u8; 3],
    media_width: u16,
    tracks: u16,
    capacity: u32,
    organizazion: [u8; 8],
    density_name: [u8; 8],
    description: [u8; 20],
}

// Returns the maximum supported drive density code
pub fn report_density<F: AsRawFd>(file: &mut F) -> Result<u8, Error> {
    let alloc_len: u16 = 8192;
    let mut sg_raw = SgRaw::new(file, alloc_len as usize)?;

    let mut cmd = Vec::new();
    cmd.extend([0x44, 0, 0, 0, 0, 0, 0]); // REPORT DENSITY SUPPORT (MEDIA=0)
    cmd.extend(alloc_len.to_be_bytes()); // alloc len
    cmd.push(0u8); // control byte

    let data = sg_raw
        .do_command(&cmd)
        .map_err(|err| format_err!("report density failed - {}", err))?;

    let mut max_density = 0u8;

    proxmox_lang::try_block!({
        let mut reader = data;

        let page_len: u16 = unsafe { reader.read_be_value()? };
        let page_len = page_len as usize;

        if (page_len + 2) > data.len() {
            bail!("invalid page length {} {}", page_len + 2, data.len());
        } else {
            // Note: Quantum hh7 returns the allocation_length instead of real data_len
            reader = &data[2..page_len + 2];
        }
        let mut reserved = [0u8; 2];
        reader.read_exact(&mut reserved)?;

        loop {
            if reader.is_empty() {
                break;
            }
            let block: DesnityDescriptorBlock = unsafe { reader.read_be_value()? };
            if block.primary_density_code > max_density {
                max_density = block.primary_density_code;
            }
        }

        Ok(())
    })
    .map_err(|err| format_err!("decode report density failed - {}", err))?;

    Ok(max_density)
}
