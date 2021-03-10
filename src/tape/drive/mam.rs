use std::collections::HashMap;
use std::convert::TryInto;
use std::os::unix::io::AsRawFd;

use anyhow::{bail, format_err, Error};
use endian_trait::Endian;

use proxmox::tools::io::ReadExt;

use crate::{
    api2::types::MamAttribute,
    tools::sgutils2::SgRaw,
    tape::{
        drive::TapeAlertFlags,
    },
};

// Read Medium auxiliary memory attributes (MAM)
// see IBM SCSI reference: https://www-01.ibm.com/support/docview.wss?uid=ssg1S7003556&aid=1

#[derive(Endian)]
#[repr(C,packed)]
struct MamAttributeHeader {
    id: u16,
    flags: u8,
    len: u16,
}

enum MamFormat {
    BINARY,
    ASCII,
    DEC,
}

static MAM_ATTRIBUTES: &[ (u16, u16, MamFormat, &str) ] = &[
    (0x00_00, 8, MamFormat::DEC, "Remaining Capacity In Partition"),
    (0x00_01, 8, MamFormat::DEC, "Maximum Capacity In Partition"),
    (0x00_02, 8, MamFormat::DEC, "Tapealert Flags"),
    (0x00_03, 8, MamFormat::DEC, "Load Count"),
    (0x00_04, 8, MamFormat::DEC, "MAM Space Remaining"),
    (0x00_05, 8, MamFormat::ASCII, "Assigning Organization"),
    (0x00_06, 1, MamFormat::BINARY, "Formatted Density Code"),
    (0x00_07, 2, MamFormat::DEC, "Initialization Count"),
    (0x00_09, 4, MamFormat::BINARY, "Volume Change Reference"),

    (0x02_0A, 40, MamFormat::ASCII, "Device Vendor/Serial Number at Last Load"),
    (0x02_0B, 40, MamFormat::ASCII, "Device Vendor/Serial Number at Load-1"),
    (0x02_0C, 40, MamFormat::ASCII, "Device Vendor/Serial Number at Load-2"),
    (0x02_0D, 40, MamFormat::ASCII, "Device Vendor/Serial Number at Load-3"),

    (0x02_20, 8, MamFormat::DEC, "Total MBytes Written in Medium Life"),
    (0x02_21, 8, MamFormat::DEC, "Total MBytes Read In Medium Life"),
    (0x02_22, 8, MamFormat::DEC, "Total MBytes Written in Current Load"),
    (0x02_23, 8, MamFormat::DEC, "Total MBytes Read in Current/Last Load"),
    (0x02_24, 8, MamFormat::BINARY, "Logical Position of First Encrypted Block"),
    (0x02_25, 8, MamFormat::BINARY, "Logical Position of First Unencrypted Block After the First Encrypted Block"),

    (0x04_00, 8, MamFormat::ASCII, "Medium Manufacturer"),
    (0x04_01, 32, MamFormat::ASCII, "Medium Serial Number"),
    (0x04_02, 4, MamFormat::DEC, "Medium Length"),
    (0x04_03, 4, MamFormat::DEC, "Medium Width"),
    (0x04_04, 8, MamFormat::ASCII, "Assigning Organization"),
    (0x04_05, 1, MamFormat::BINARY, "Medium Density Code"),
    (0x04_06, 8, MamFormat::ASCII, "Medium Manufacture Date"),
    (0x04_07, 8, MamFormat::DEC, "MAM Capacity"),
    (0x04_08, 1, MamFormat::BINARY, "Medium Type"),
    (0x04_09, 2, MamFormat::BINARY, "Medium Type Information"),
    (0x04_0B, 10, MamFormat::BINARY, "Supported Density Codes"),

    (0x08_00, 8, MamFormat::ASCII, "Application Vendor"),
    (0x08_01, 32, MamFormat::ASCII, "Application Name"),
    (0x08_02, 8, MamFormat::ASCII, "Application Version"),
    (0x08_03, 160, MamFormat::ASCII, "User Medium Text Label"),
    (0x08_04, 12, MamFormat::ASCII, "Date And Time Last Written"),
    (0x08_05, 1, MamFormat::BINARY, "Text Localization Identifier"),
    (0x08_06, 32, MamFormat::ASCII, "Barcode"),
    (0x08_07, 80, MamFormat::ASCII, "Owning Host Textual Name"),
    (0x08_08, 160, MamFormat::ASCII, "Media Pool"),
    (0x08_0B, 16, MamFormat::ASCII, "Application Format Version"),
    (0x08_0C, 50, MamFormat::ASCII, "Volume Coherency Information"),
    (0x08_20, 36, MamFormat::ASCII, "Medium Globally Unique Identifier"),
    (0x08_21, 36, MamFormat::ASCII, "Media Pool Globally Unique Identifier"),

    (0x10_00, 28,  MamFormat::BINARY, "Unique Cartridge Identify (UCI)"),
    (0x10_01, 24,  MamFormat::BINARY, "Alternate Unique Cartridge Identify (Alt-UCI)"),

];

lazy_static::lazy_static!{

    static ref MAM_ATTRIBUTE_NAMES: HashMap<u16, &'static (u16, u16, MamFormat, &'static str)> = {
        let mut map = HashMap::new();

        for entry in MAM_ATTRIBUTES {
            map.insert(entry.0, entry);
        }

        map
    };
}

fn read_tape_mam<F: AsRawFd>(file: &mut F) -> Result<Vec<u8>, Error> {

    let alloc_len: u32 = 32*1024;
    let mut sg_raw = SgRaw::new(file, alloc_len as usize)?;

    let mut cmd = Vec::new();
    cmd.extend(&[0x8c, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8]);
    cmd.extend(&[0u8, 0u8]); // first attribute
    cmd.extend(&alloc_len.to_be_bytes()); // alloc len
    cmd.extend(&[0u8, 0u8]);

    sg_raw.do_command(&cmd)
        .map_err(|err| format_err!("read cartidge memory failed - {}", err))
        .map(|v| v.to_vec())
}

/// Read Medium auxiliary memory attributes (cartridge memory) using raw SCSI command.
pub fn read_mam_attributes<F: AsRawFd>(file: &mut F) -> Result<Vec<MamAttribute>, Error> {

    let data = read_tape_mam(file)?;

    decode_mam_attributes(&data)
}

fn decode_mam_attributes(data: &[u8]) -> Result<Vec<MamAttribute>, Error> {

    let mut reader = &data[..];

    let data_len: u32 = unsafe { reader.read_be_value()? };

    let expected_len = data_len as usize;


    if reader.len() < expected_len {
        bail!("read_mam_attributes: got unexpected data len ({} != {})", reader.len(), expected_len);
    } else if reader.len() > expected_len {
        // Note: Quantum hh7 returns the allocation_length instead of real data_len
        reader = &data[4..expected_len+4];
    }

    let mut list = Vec::new();

    loop {
        if reader.is_empty() {
            break;
        }
        let head: MamAttributeHeader =  unsafe { reader.read_be_value()? };
        //println!("GOT ID {:04X} {:08b} {}", head.id, head.flags, head.len);

        let head_id = head.id;

        let data = if head.len > 0 {
            reader.read_exact_allocated(head.len as usize)?
        } else {
            Vec::new()
        };

        if let Some(info) = MAM_ATTRIBUTE_NAMES.get(&head_id) {
            if info.1 == head.len {
                let value = match info.2 {
                    MamFormat::ASCII => String::from_utf8_lossy(&data).to_string(),
                    MamFormat::DEC => {
                        if info.1 == 2 {
                            format!("{}", u16::from_be_bytes(data[0..2].try_into()?))
                        } else if info.1 == 4 {
                            format!("{}", u32::from_be_bytes(data[0..4].try_into()?))
                        } else if info.1 == 8 {
                            if head_id == 2 { // Tape Alert Flags
                                let value = u64::from_be_bytes(data[0..8].try_into()?);
                                let flags = TapeAlertFlags::from_bits_truncate(value);
                                format!("{:?}", flags)
                            } else {
                                format!("{}", u64::from_be_bytes(data[0..8].try_into()?))
                            }
                        } else {
                            unreachable!();
                        }
                    },
                    MamFormat::BINARY => proxmox::tools::digest_to_hex(&data),
                };
                list.push(MamAttribute {
                    id: head_id,
                    name: info.3.to_string(),
                    value,
                });
            } else {
                eprintln!("read_mam_attributes: got starnge data len for id {:04X}", head_id);
            }
        } else {
            // skip unknown IDs
        }
    }
    Ok(list)
}

/// Media Usage Information from Cartridge Memory
pub struct MediaUsageInfo {
    pub manufactured: i64,
    pub bytes_read: u64,
    pub bytes_written: u64,
}

/// Extract Media Usage Information from Cartridge Memory
pub fn mam_extract_media_usage(mam: &[MamAttribute]) -> Result<MediaUsageInfo, Error> {

   let manufactured: i64 = match mam.iter().find(|v| v.id == 0x04_06).map(|v| v.value.clone()) {
        Some(date_str) => {
            if date_str.len() != 8 {
                bail!("unable to parse 'Medium Manufacture Date' - wrong length");
            }
            let year: i32 = date_str[..4].parse()?;
            let mon: i32 = date_str[4..6].parse()?;
            let mday: i32 = date_str[6..8].parse()?;

            use proxmox::tools::time::TmEditor;
            let mut t = TmEditor::new(true);
            t.set_year(year)?;
            t.set_mon(mon)?;
            t.set_mday(mday)?;

            t.into_epoch()?
        }
        None => bail!("unable to read MAM 'Medium Manufacture Date'"),
    };

    let bytes_written: u64 = match mam.iter().find(|v| v.id == 0x02_20).map(|v| v.value.clone()) {
        Some(read_str) => read_str.parse::<u64>()? * 1024*1024,
        None => bail!("unable to read MAM 'Total MBytes Written In Medium Life'"),
    };

    let bytes_read: u64 = match mam.iter().find(|v| v.id == 0x02_21).map(|v| v.value.clone()) {
        Some(read_str) => read_str.parse::<u64>()? * 1024*1024,
        None => bail!("unable to read MAM 'Total MBytes Read In Medium Life'"),
    };

    Ok(MediaUsageInfo { manufactured, bytes_written, bytes_read })
}
