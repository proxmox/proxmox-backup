use std::collections::HashMap;
use std::os::unix::io::AsRawFd;

use anyhow::{bail, format_err, Error};
use endian_trait::Endian;

use proxmox_io::ReadExt;

use pbs_api_types::MamAttribute;

use crate::sgutils2::SgRaw;

use super::TapeAlertFlags;

// Read Medium auxiliary memory attributes (MAM)
// see IBM SCSI reference: https://www.ibm.com/support/pages/node/6490249

#[derive(Endian)]
#[repr(C, packed)]
struct MamAttributeHeader {
    id: u16,
    flags: u8,
    len: u16,
}

#[allow(clippy::upper_case_acronyms)]
enum MamFormat {
    BINARY,
    ASCII,
    DEC,
}

struct MamType {
    pub id: u16,
    pub len: u16,
    pub format: MamFormat,
    pub description: &'static str,
}

impl MamType {
    const fn new(id: u16, len: u16, format: MamFormat, description: &'static str) -> Self {
        MamType {
            id,
            len,
            format,
            description,
        }
    }
    const fn bin(id: u16, len: u16, description: &'static str) -> Self {
        Self::new(id, len, MamFormat::BINARY, description)
    }
    const fn ascii(id: u16, len: u16, description: &'static str) -> Self {
        Self::new(id, len, MamFormat::ASCII, description)
    }
    const fn dec(id: u16, len: u16, description: &'static str) -> Self {
        Self::new(id, len, MamFormat::DEC, description)
    }
}

static MAM_ATTRIBUTES: &[MamType] = &[
    MamType::dec(0x00_00, 8, "Remaining Capacity In Partition"),
    MamType::dec(0x00_01, 8, "Maximum Capacity In Partition"),
    MamType::dec(0x00_02, 8, "Tapealert Flags"),
    MamType::dec(0x00_03, 8, "Load Count"),
    MamType::dec(0x00_04, 8, "MAM Space Remaining"),
    MamType::ascii(0x00_05, 8, "Assigning Organization"),
    MamType::bin(0x00_06, 1, "Formatted Density Code"),
    MamType::dec(0x00_07, 2, "Initialization Count"),
    MamType::bin(0x00_09, 4, "Volume Change Reference"),
    MamType::ascii(0x02_0A, 40, "Device Vendor/Serial Number at Last Load"),
    MamType::ascii(0x02_0B, 40, "Device Vendor/Serial Number at Load-1"),
    MamType::ascii(0x02_0C, 40, "Device Vendor/Serial Number at Load-2"),
    MamType::ascii(0x02_0D, 40, "Device Vendor/Serial Number at Load-3"),
    MamType::dec(0x02_20, 8, "Total MBytes Written in Medium Life"),
    MamType::dec(0x02_21, 8, "Total MBytes Read In Medium Life"),
    MamType::dec(0x02_22, 8, "Total MBytes Written in Current Load"),
    MamType::dec(0x02_23, 8, "Total MBytes Read in Current/Last Load"),
    MamType::bin(0x02_24, 8, "Logical Position of First Encrypted Block"),
    MamType::bin(
        0x02_25,
        8,
        "Logical Position of First Unencrypted Block After the First Encrypted Block",
    ),
    MamType::ascii(0x04_00, 8, "Medium Manufacturer"),
    MamType::ascii(0x04_01, 32, "Medium Serial Number"),
    MamType::dec(0x04_02, 4, "Medium Length"),
    MamType::dec(0x04_03, 4, "Medium Width"),
    MamType::ascii(0x04_04, 8, "Assigning Organization"),
    MamType::bin(0x04_05, 1, "Medium Density Code"),
    MamType::ascii(0x04_06, 8, "Medium Manufacture Date"),
    MamType::dec(0x04_07, 8, "MAM Capacity"),
    MamType::bin(0x04_08, 1, "Medium Type"),
    MamType::bin(0x04_09, 2, "Medium Type Information"),
    MamType::bin(0x04_0B, 10, "Supported Density Codes"),
    MamType::ascii(0x08_00, 8, "Application Vendor"),
    MamType::ascii(0x08_01, 32, "Application Name"),
    MamType::ascii(0x08_02, 8, "Application Version"),
    MamType::ascii(0x08_03, 160, "User Medium Text Label"),
    MamType::ascii(0x08_04, 12, "Date And Time Last Written"),
    MamType::bin(0x08_05, 1, "Text Localization Identifier"),
    MamType::ascii(0x08_06, 32, "Barcode"),
    MamType::ascii(0x08_07, 80, "Owning Host Textual Name"),
    MamType::ascii(0x08_08, 160, "Media Pool"),
    MamType::ascii(0x08_0B, 16, "Application Format Version"),
    // length for vol. coherency is not specified for IBM, and HP says 23-n
    MamType::bin(0x08_0C, 0, "Volume Coherency Information"),
    MamType::bin(0x08_20, 36, "Medium Globally Unique Identifier"),
    MamType::bin(0x08_21, 36, "Media Pool Globally Unique Identifier"),
    MamType::bin(0x10_00, 28, "Unique Cartridge Identify (UCI)"),
    MamType::bin(0x10_01, 24, "Alternate Unique Cartridge Identify (Alt-UCI)"),
];

lazy_static::lazy_static! {

    static ref MAM_ATTRIBUTE_NAMES: HashMap<u16, &'static MamType> = {
        let mut map = HashMap::new();

        for entry in MAM_ATTRIBUTES {
            map.insert(entry.id, entry);
        }

        map
    };
}

fn read_tape_mam<F: AsRawFd>(file: &mut F) -> Result<Vec<u8>, Error> {
    let alloc_len: u32 = 32 * 1024;
    let mut sg_raw = SgRaw::new(file, alloc_len as usize)?;

    let mut cmd = Vec::new();
    cmd.extend([0x8c, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8]);
    cmd.extend([0u8, 0u8]); // first attribute
    cmd.extend(alloc_len.to_be_bytes()); // alloc len
    cmd.extend([0u8, 0u8]);

    sg_raw
        .do_command(&cmd)
        .map_err(|err| format_err!("read cartidge memory failed - {}", err))
        .map(|v| v.to_vec())
}

/// Read Medium auxiliary memory attributes (cartridge memory) using raw SCSI command.
pub fn read_mam_attributes<F: AsRawFd>(file: &mut F) -> Result<Vec<MamAttribute>, Error> {
    let data = read_tape_mam(file)?;

    decode_mam_attributes(&data)
}

fn decode_mam_attributes(data: &[u8]) -> Result<Vec<MamAttribute>, Error> {
    let mut reader = data;

    let data_len: u32 = unsafe { reader.read_be_value()? };
    let expected_len = data_len as usize;

    use std::cmp::Ordering;
    match reader.len().cmp(&expected_len) {
        Ordering::Less => bail!(
            "read_mam_attributes: got unexpected data len ({} != {expected_len})",
            reader.len()
        ),
        Ordering::Greater => {
            // Note: Quantum hh7 returns the allocation_length instead of real data_len
            reader = &data[4..expected_len + 4];
        }
        _ => (),
    }

    let mut list = Vec::new();

    loop {
        if reader.is_empty() {
            break;
        }
        let head: MamAttributeHeader = unsafe { reader.read_be_value()? };
        //println!("GOT ID {:04X} {:08b} {}", head.id, head.flags, head.len);

        let head_id = head.id;

        let data = if head.len > 0 {
            reader.read_exact_allocated(head.len as usize)?
        } else {
            Vec::new()
        };

        let info = match MAM_ATTRIBUTE_NAMES.get(&head_id) {
            None => continue, // skip unknown IDs
            Some(info) => info,
        };
        if info.len == 0 || info.len == head.len {
            let value = match info.format {
                MamFormat::ASCII => String::from_utf8_lossy(&data).to_string(),
                MamFormat::DEC => {
                    if info.len == 2 {
                        format!("{}", u16::from_be_bytes(data[0..2].try_into()?))
                    } else if info.len == 4 {
                        format!("{}", u32::from_be_bytes(data[0..4].try_into()?))
                    } else if info.len == 8 {
                        if head_id == 2 {
                            // Tape Alert Flags
                            let value = u64::from_be_bytes(data[0..8].try_into()?);
                            let flags = TapeAlertFlags::from_bits_truncate(value);
                            format!("{:?}", flags)
                        } else {
                            format!("{}", u64::from_be_bytes(data[0..8].try_into()?))
                        }
                    } else {
                        bail!("unexpected MAM attribute length {}", info.len);
                    }
                }
                MamFormat::BINARY => hex::encode(&data),
            };
            list.push(MamAttribute {
                id: head_id,
                name: info.description.to_string(),
                value,
            });
        } else {
            eprintln!("read_mam_attributes: got strange data len for id {head_id:04X}");
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
    let manufactured: i64 = match mam
        .iter()
        .find(|v| v.id == 0x04_06)
        .map(|v| v.value.clone())
    {
        Some(date_str) => {
            if date_str.len() != 8 {
                bail!("unable to parse 'Medium Manufacture Date' - wrong length");
            }
            let year: i32 = date_str[..4].parse()?;
            let mon: i32 = date_str[4..6].parse()?;
            let mday: i32 = date_str[6..8].parse()?;

            use proxmox_time::TmEditor;
            let mut t = TmEditor::new(true);
            t.set_year(year)?;
            t.set_mon(mon)?;
            t.set_mday(mday)?;

            t.into_epoch()?
        }
        None => bail!("unable to read MAM 'Medium Manufacture Date'"),
    };

    let bytes_written: u64 = match mam
        .iter()
        .find(|v| v.id == 0x02_20)
        .map(|v| v.value.clone())
    {
        Some(read_str) => read_str.parse::<u64>()? * 1024 * 1024,
        None => bail!("unable to read MAM 'Total MBytes Written In Medium Life'"),
    };

    let bytes_read: u64 = match mam
        .iter()
        .find(|v| v.id == 0x02_21)
        .map(|v| v.value.clone())
    {
        Some(read_str) => read_str.parse::<u64>()? * 1024 * 1024,
        None => bail!("unable to read MAM 'Total MBytes Read In Medium Life'"),
    };

    Ok(MediaUsageInfo {
        manufactured,
        bytes_written,
        bytes_read,
    })
}
