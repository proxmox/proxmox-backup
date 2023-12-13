//! SCSI changer implementation using libsgutil2
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::Read;
use std::os::unix::prelude::AsRawFd;
use std::path::Path;

use anyhow::{bail, format_err, Error};
use endian_trait::Endian;

use proxmox_io::ReadExt;

use pbs_api_types::ScsiTapeChanger;

use crate::{
    sgutils2::{scsi_ascii_to_string, scsi_inquiry, ScsiError, SgRaw, SENSE_KEY_NOT_READY},
    DriveStatus, ElementStatus, MtxStatus, StorageElementStatus, TransportElementStatus,
};

const SCSI_CHANGER_DEFAULT_TIMEOUT: usize = 60 * 5; // 5 minutes
const SCSI_VOLUME_TAG_LEN: usize = 36;

/// Initialize element status (Inventory)
pub fn initialize_element_status<F: AsRawFd>(file: &mut F) -> Result<(), Error> {
    let mut sg_raw = SgRaw::new(file, 64)?;

    // like mtx(1), set a very long timeout (30 minutes)
    sg_raw.set_timeout(30 * 60);

    let cmd = &[0x07, 0, 0, 0, 0, 0]; // INITIALIZE ELEMENT STATUS (07h)

    sg_raw
        .do_command(cmd)
        .map_err(|err| format_err!("initializte element status (07h) failed - {}", err))?;

    Ok(())
}

#[repr(C, packed)]
#[derive(Endian)]
struct AddressAssignmentPage {
    data_len: u8,
    reserved1: u8,
    reserved2: u8,
    block_descriptor_len: u8,

    page_code: u8,
    additional_page_len: u8,
    first_transport_element_address: u16,
    transport_element_count: u16,
    first_storage_element_address: u16,
    storage_element_count: u16,
    first_import_export_element_address: u16,
    import_export_element_count: u16,
    first_tranfer_element_address: u16,
    transfer_element_count: u16,
    reserved22: u8,
    reserved23: u8,
}

/// Execute scsi commands, optionally repeat the command until
/// successful or timeout (sleep 1 second between invovations)
///
/// Timeout is 5 seconds. If the device reports "Not Ready - becoming
/// ready", we wait up to 5 minutes.
///
/// Skipped errors are printed on stderr.
fn execute_scsi_command<F: AsRawFd>(
    sg_raw: &mut SgRaw<F>,
    cmd: &[u8],
    error_prefix: &str,
    retry: bool,
) -> Result<Vec<u8>, Error> {
    let start = std::time::SystemTime::now();

    let mut last_msg: Option<String> = None;

    let mut timeout = std::time::Duration::new(5, 0); // short timeout by default

    loop {
        match sg_raw.do_command(cmd) {
            Ok(data) => return Ok(data.to_vec()),
            Err(err) if !retry => bail!("{} failed: {}", error_prefix, err),
            Err(err) => {
                let msg = err.to_string();
                if let Some(ref last) = last_msg {
                    if &msg != last {
                        log::error!("{}", err);
                        last_msg = Some(msg);
                    }
                } else {
                    log::error!("{}", err);
                    last_msg = Some(msg);
                }

                if let ScsiError::Sense(ref sense) = err {
                    // Not Ready - becoming ready
                    if sense.sense_key == SENSE_KEY_NOT_READY
                        && sense.asc == 0x04
                        && sense.ascq == 1
                    {
                        // wait up to 5 minutes, long enough to finish inventorize
                        timeout = std::time::Duration::new(5 * 60, 0);
                    }
                }

                if start.elapsed()? > timeout {
                    bail!("{} failed: {}", error_prefix, err);
                }

                std::thread::sleep(std::time::Duration::new(1, 0));
                continue; // try again
            }
        }
    }
}

fn read_element_address_assignment<F: AsRawFd>(
    file: &mut F,
) -> Result<AddressAssignmentPage, Error> {
    let allocation_len: u8 = u8::MAX;
    let mut sg_raw = SgRaw::new(file, allocation_len as usize)?;
    sg_raw.set_timeout(SCSI_CHANGER_DEFAULT_TIMEOUT);

    let cmd = &[
        0x1A, // MODE SENSE6 (1Ah)
        0x08, // DBD=1 (The Disable Block Descriptors)
        0x1D, // Element Address Assignment Page
        0,
        allocation_len, // allocation len
        0,              //control
    ];

    let data = execute_scsi_command(&mut sg_raw, cmd, "read element address assignment", true)?;

    proxmox_lang::try_block!({
        let mut reader = &data[..];
        let page: AddressAssignmentPage = unsafe { reader.read_be_value()? };

        if page.data_len != 23 {
            bail!("got unexpected page len ({} != 23)", page.data_len);
        }

        Ok(page)
    })
    .map_err(|err: Error| format_err!("decode element address assignment page failed - {}", err))
}

#[allow(clippy::vec_init_then_push)]
fn scsi_move_medium_cdb(
    medium_transport_address: u16,
    source_element_address: u16,
    destination_element_address: u16,
) -> Vec<u8> {
    let mut cmd = Vec::new();
    cmd.push(0xA5); // MOVE MEDIUM (A5h)
    cmd.push(0); // reserved
    cmd.extend(medium_transport_address.to_be_bytes());
    cmd.extend(source_element_address.to_be_bytes());
    cmd.extend(destination_element_address.to_be_bytes());
    cmd.push(0); // reserved
    cmd.push(0); // reserved
    cmd.push(0); // Invert=0
    cmd.push(0); // control

    cmd
}

/// Load media from storage slot into drive
pub fn load_slot(file: &mut File, from_slot: u64, drivenum: u64) -> Result<(), Error> {
    let status = read_element_status(file)?;

    let transport_address = status.transport_address();
    let source_element_address = status.slot_address(from_slot)?;
    let drive_element_address = status.drive_address(drivenum)?;

    let cmd = scsi_move_medium_cdb(
        transport_address,
        source_element_address,
        drive_element_address,
    );

    let mut sg_raw = SgRaw::new(file, 64)?;
    sg_raw.set_timeout(SCSI_CHANGER_DEFAULT_TIMEOUT);

    sg_raw
        .do_command(&cmd)
        .map_err(|err| format_err!("load drive failed - {}", err))?;

    Ok(())
}

/// Unload media from drive into a storage slot
pub fn unload(file: &mut File, to_slot: u64, drivenum: u64) -> Result<(), Error> {
    let status = read_element_status(file)?;

    let transport_address = status.transport_address();
    let target_element_address = status.slot_address(to_slot)?;
    let drive_element_address = status.drive_address(drivenum)?;

    let cmd = scsi_move_medium_cdb(
        transport_address,
        drive_element_address,
        target_element_address,
    );

    let mut sg_raw = SgRaw::new(file, 64)?;
    sg_raw.set_timeout(SCSI_CHANGER_DEFAULT_TIMEOUT);

    sg_raw
        .do_command(&cmd)
        .map_err(|err| format_err!("unload drive failed - {}", err))?;

    Ok(())
}

/// Transfer medium from one storage slot to another
pub fn transfer_medium<F: AsRawFd>(
    file: &mut F,
    from_slot: u64,
    to_slot: u64,
) -> Result<(), Error> {
    let status = read_element_status(file)?;

    let transport_address = status.transport_address();
    let source_element_address = status.slot_address(from_slot)?;
    let target_element_address = status.slot_address(to_slot)?;

    let cmd = scsi_move_medium_cdb(
        transport_address,
        source_element_address,
        target_element_address,
    );

    let mut sg_raw = SgRaw::new(file, 64)?;
    sg_raw.set_timeout(SCSI_CHANGER_DEFAULT_TIMEOUT);

    sg_raw.do_command(&cmd).map_err(|err| {
        format_err!(
            "transfer medium from slot {} to slot {} failed - {}",
            from_slot,
            to_slot,
            err
        )
    })?;

    Ok(())
}

#[derive(Clone, Copy, Debug)]
enum ElementType {
    MediumTransport,
    Storage,
    ImportExport,
    DataTransfer,
    DataTransferWithDVCID,
}

impl ElementType {
    fn byte1(&self) -> u8 {
        let volume_tag_bit = 1u8 << 4;
        match *self {
            ElementType::MediumTransport => volume_tag_bit | 1,
            ElementType::Storage => volume_tag_bit | 2,
            ElementType::ImportExport => volume_tag_bit | 3,
            ElementType::DataTransfer => volume_tag_bit | 4,
            // some changers cannot get voltag + dvcid at the same time
            ElementType::DataTransferWithDVCID => 4,
        }
    }

    fn byte6(&self) -> u8 {
        match *self {
            ElementType::DataTransferWithDVCID => 0b001, //  Mixed=0,CurData=0,DVCID=1
            _ => 0b000,                                  // Mixed=0,CurData=0,DVCID=0
        }
    }
}

#[allow(clippy::vec_init_then_push)]
fn scsi_read_element_status_cdb(
    start_element_address: u16,
    number_of_elements: u16,
    element_type: ElementType,
    allocation_len: u32,
) -> Vec<u8> {
    let mut cmd = Vec::new();
    cmd.push(0xB8); // READ ELEMENT STATUS (B8h)
    cmd.push(element_type.byte1());
    cmd.extend(start_element_address.to_be_bytes());

    cmd.extend(number_of_elements.to_be_bytes());
    cmd.push(element_type.byte6());
    cmd.extend(&allocation_len.to_be_bytes()[1..4]);
    cmd.push(0);
    cmd.push(0);

    cmd
}

// query a single element type from the changer
fn get_element<F: AsRawFd>(
    sg_raw: &mut SgRaw<F>,
    element_type: ElementType,
    allocation_len: u32,
    mut retry: bool,
) -> Result<DecodedStatusPage, Error> {
    let mut start_element_address = 0;
    let number_of_elements: u16 = 1000; // some changers limit the query

    let mut result = DecodedStatusPage {
        last_element_address: None,
        transports: Vec::new(),
        drives: Vec::new(),
        storage_slots: Vec::new(),
        import_export_slots: Vec::new(),
    };

    loop {
        let cmd = scsi_read_element_status_cdb(
            start_element_address,
            number_of_elements,
            element_type,
            allocation_len,
        );

        let data = execute_scsi_command(sg_raw, &cmd, "read element status (B8h)", retry)?;

        let page = decode_element_status_page(&data, start_element_address).map_err(|err| {
            format_err!("decode element status for {element_type:?} on {start_element_address} failed - {err}")
        })?;

        retry = false; // only retry the first command

        let returned_number_of_elements = page.transports.len()
            + page.drives.len()
            + page.storage_slots.len()
            + page.import_export_slots.len();

        result.transports.extend(page.transports);
        result.drives.extend(page.drives);
        result.storage_slots.extend(page.storage_slots);
        result.import_export_slots.extend(page.import_export_slots);
        result.last_element_address = page.last_element_address;

        if let Some(last_element_address) = page.last_element_address {
            if last_element_address < start_element_address {
                bail!("got strange element address");
            }
            if returned_number_of_elements >= (number_of_elements as usize) {
                start_element_address = last_element_address + 1;
                continue; // we possibly have to read additional elements
            }
        }
        break;
    }

    Ok(result)
}

/// Read element status.
pub fn read_element_status<F: AsRawFd>(file: &mut F) -> Result<MtxStatus, Error> {
    let inquiry = scsi_inquiry(file)?;

    if inquiry.peripheral_type != 8 {
        bail!("wrong device type (not a scsi changer device)");
    }

    // first, request address assignment (used for sanity checks)
    let setup = read_element_address_assignment(file)?;

    let allocation_len: u32 = 0xFFFF; // some changer only use the lower 2 bytes

    let mut sg_raw = SgRaw::new(file, allocation_len as usize)?;
    sg_raw.set_timeout(SCSI_CHANGER_DEFAULT_TIMEOUT);

    let mut drives = Vec::new();
    let mut storage_slots = Vec::new();
    let mut import_export_slots = Vec::new();
    let mut transports = Vec::new();

    let page = get_element(&mut sg_raw, ElementType::Storage, allocation_len, true)?;
    storage_slots.extend(page.storage_slots);

    let page = get_element(
        &mut sg_raw,
        ElementType::ImportExport,
        allocation_len,
        false,
    )?;
    import_export_slots.extend(page.import_export_slots);

    let page = get_element(
        &mut sg_raw,
        ElementType::DataTransfer,
        allocation_len,
        false,
    )?;
    drives.extend(page.drives);

    // get the serial + vendor + model,
    // some changer require this to be an extra scsi command
    // some changers don't support this
    if let Ok(page) = get_element(
        &mut sg_raw,
        ElementType::DataTransferWithDVCID,
        allocation_len,
        false,
    ) {
        // should be in same order and same count, but be on the safe side.
        // there should not be too many drives normally
        for drive in drives.iter_mut() {
            for drive2 in &page.drives {
                if drive2.element_address == drive.element_address {
                    drive.vendor = drive2.vendor.clone();
                    drive.model = drive2.model.clone();
                    drive.drive_serial_number = drive2.drive_serial_number.clone();
                }
            }
        }
    }

    let page = get_element(
        &mut sg_raw,
        ElementType::MediumTransport,
        allocation_len,
        false,
    )?;
    transports.extend(page.transports);

    let transport_count = setup.transport_element_count as usize;
    let storage_count = setup.storage_element_count as usize;
    let import_export_count = setup.import_export_element_count as usize;
    let transfer_count = setup.transfer_element_count as usize;

    if transport_count != transports.len() {
        bail!(
            "got wrong number of transport elements: expoected {}, got{}",
            transport_count,
            transports.len()
        );
    }
    if storage_count != storage_slots.len() {
        bail!(
            "got wrong number of storage elements: expected {}, got {}",
            storage_count,
            storage_slots.len(),
        );
    }
    if import_export_count != import_export_slots.len() {
        bail!(
            "got wrong number of import/export elements: expected {}, got {}",
            import_export_count,
            import_export_slots.len(),
        );
    }
    if transfer_count != drives.len() {
        bail!(
            "got wrong number of transfer elements: expected {}, got {}",
            transfer_count,
            drives.len(),
        );
    }

    // create same virtual slot order as mtx(1)
    // - storage slots first
    // - import export slots at the end
    let mut slots = storage_slots;
    slots.extend(import_export_slots);

    let mut status = MtxStatus {
        transports,
        drives,
        slots,
    };

    // sanity checks
    if status.drives.is_empty() {
        bail!("no data transfer elements reported");
    }
    if status.slots.is_empty() {
        bail!("no storage elements reported");
    }

    // compute virtual storage slot to element_address map
    let mut slot_map = HashMap::new();
    for (i, slot) in status.slots.iter().enumerate() {
        slot_map.insert(slot.element_address, (i + 1) as u64);
    }

    // translate element addresses in loaded_lot
    for drive in status.drives.iter_mut() {
        if let Some(source_address) = drive.loaded_slot {
            let source_address = source_address as u16;
            drive.loaded_slot = slot_map.get(&source_address).copied();
        }
    }

    Ok(status)
}

/// Read status and map import-export slots from config
pub fn status(config: &ScsiTapeChanger) -> Result<MtxStatus, Error> {
    let path = &config.path;

    let mut file = open(path).map_err(|err| format_err!("error opening '{}': {}", path, err))?;
    let mut status = read_element_status(&mut file)
        .map_err(|err| format_err!("error reading element status: {}", err))?;

    status.mark_import_export_slots(config)?;

    Ok(status)
}

#[repr(C, packed)]
#[derive(Endian)]
struct ElementStatusHeader {
    first_element_address_reported: u16,
    number_of_elements_available: u16,
    reserved: u8,
    byte_count_of_report_available: [u8; 3],
}

#[repr(C, packed)]
#[derive(Endian)]
struct SubHeader {
    element_type_code: u8,
    flags: u8,
    descriptor_length: u16,
    reserved: u8,
    byte_count_of_descriptor_data_available: [u8; 3],
}

impl SubHeader {
    fn parse_optional_volume_tag<R: Read>(
        &self,
        reader: &mut R,
        full: bool,
    ) -> Result<Option<String>, Error> {
        if (self.flags & 128) != 0 {
            // has PVolTag
            let tmp = reader.read_exact_allocated(SCSI_VOLUME_TAG_LEN)?;
            if full {
                let volume_tag = scsi_ascii_to_string(&tmp);
                return Ok(Some(volume_tag));
            }
        }
        Ok(None)
    }

    // AFAIK, tape changer do not use AlternateVolumeTag
    // but parse anyways, just to be sure
    fn skip_alternate_volume_tag<R: Read>(&self, reader: &mut R) -> Result<Option<String>, Error> {
        if (self.flags & 64) != 0 {
            // has AVolTag
            let _tmp = reader.read_exact_allocated(SCSI_VOLUME_TAG_LEN)?;
        }

        Ok(None)
    }
}

#[repr(C, packed)]
#[derive(Endian)]
struct TransportDescriptor {
    // Robot/Griper
    element_address: u16,
    flags1: u8,
    reserved_3: u8,
    additional_sense_code: u8,
    additional_sense_code_qualifier: u8,
    reserved_6: [u8; 3],
    flags2: u8,
    source_storage_element_address: u16,
    // volume tag and Mixed media descriptor follows (depends on flags)
}

#[repr(C, packed)]
#[derive(Endian)]
struct TransferDescriptor {
    // Tape drive
    element_address: u16,
    flags1: u8,
    reserved_3: u8,
    additional_sense_code: u8,
    additional_sense_code_qualifier: u8,
    id_valid: u8,
    scsi_bus_address: u8,
    reserved_8: u8,
    flags2: u8,
    source_storage_element_address: u16,
    // volume tag, drive identifier and Mixed media descriptor follows
    // (depends on flags)
}

#[repr(C, packed)]
#[derive(Endian)]
struct DvcidHead {
    // Drive Identifier Header
    code_set: u8,
    identifier_type: u8,
    reserved: u8,
    identifier_len: u8,
    // Identifier follows
}

#[repr(C, packed)]
#[derive(Endian)]
struct StorageDescriptor {
    // Mail Slot
    element_address: u16,
    flags1: u8,
    reserved_3: u8,
    additional_sense_code: u8,
    additional_sense_code_qualifier: u8,
    reserved_6: [u8; 3],
    flags2: u8,
    source_storage_element_address: u16,
    // volume tag and Mixed media descriptor follows (depends on flags)
}

struct DecodedStatusPage {
    last_element_address: Option<u16>,
    transports: Vec<TransportElementStatus>,
    drives: Vec<DriveStatus>,
    storage_slots: Vec<StorageElementStatus>,
    import_export_slots: Vec<StorageElementStatus>,
}

fn create_element_status(full: bool, volume_tag: Option<String>) -> ElementStatus {
    if full {
        if let Some(volume_tag) = volume_tag {
            ElementStatus::VolumeTag(volume_tag)
        } else {
            ElementStatus::Full
        }
    } else {
        ElementStatus::Empty
    }
}

struct DvcidInfo {
    vendor: Option<String>,
    model: Option<String>,
    serial: Option<String>,
}

fn decode_dvcid_info<R: Read>(reader: &mut R) -> Result<DvcidInfo, Error> {
    let dvcid: DvcidHead = unsafe { reader.read_be_value()? };

    let (serial, vendor, model) = match (dvcid.code_set, dvcid.identifier_type) {
        (2, 0) => {
            // Serial number only (Quantum Superloader3 uses this)
            let serial = reader.read_exact_allocated(dvcid.identifier_len as usize)?;
            let serial = scsi_ascii_to_string(&serial);
            (Some(serial), None, None)
        }
        (2, 1) => {
            if dvcid.identifier_len != 34 {
                bail!("got wrong DVCID length");
            }
            let vendor = reader.read_exact_allocated(8)?;
            let vendor = scsi_ascii_to_string(&vendor);
            let model = reader.read_exact_allocated(16)?;
            let model = scsi_ascii_to_string(&model);
            let serial = reader.read_exact_allocated(10)?;
            let serial = scsi_ascii_to_string(&serial);
            (Some(serial), Some(vendor), Some(model))
        }
        _ => (None, None, None),
    };

    Ok(DvcidInfo {
        vendor,
        model,
        serial,
    })
}

fn decode_element_status_page(
    data: &[u8],
    start_element_address: u16,
) -> Result<DecodedStatusPage, Error> {
    let mut result = DecodedStatusPage {
        last_element_address: None,
        transports: Vec::new(),
        drives: Vec::new(),
        storage_slots: Vec::new(),
        import_export_slots: Vec::new(),
    };

    let mut reader = data;

    let head: ElementStatusHeader = unsafe { reader.read_be_value()? };

    if head.number_of_elements_available == 0 {
        return Ok(result);
    }

    if head.first_element_address_reported < start_element_address {
        bail!("got wrong first_element_address_reported"); // sanity check
    }

    let len = head.byte_count_of_report_available;
    let len = ((len[0] as usize) << 16) + ((len[1] as usize) << 8) + (len[2] as usize);

    use std::cmp::Ordering;
    match len.cmp(&reader.len()) {
        Ordering::Less => reader = &reader[..len],
        Ordering::Greater => bail!(
            "wrong amount of data: expected {}, got {}",
            len,
            reader.len()
        ),
        _ => (),
    }

    loop {
        if reader.is_empty() {
            break;
        }

        let subhead: SubHeader = unsafe { reader.read_be_value()? };

        let len = subhead.byte_count_of_descriptor_data_available;
        let mut len = ((len[0] as usize) << 16) + ((len[1] as usize) << 8) + (len[2] as usize);
        if len > reader.len() {
            len = reader.len();
        }

        let descr_data = reader.read_exact_allocated(len)?;

        let descr_len = subhead.descriptor_length as usize;

        if descr_len == 0 {
            bail!("got elements, but descriptor length 0");
        }

        for descriptor in descr_data.chunks_exact(descr_len) {
            let mut reader = descriptor;

            match subhead.element_type_code {
                1 => {
                    let desc: TransportDescriptor = unsafe { reader.read_be_value()? };

                    let full = (desc.flags1 & 1) != 0;
                    let volume_tag = subhead.parse_optional_volume_tag(&mut reader, full)?;

                    subhead.skip_alternate_volume_tag(&mut reader)?;

                    result.last_element_address = Some(desc.element_address);

                    let status = TransportElementStatus {
                        status: create_element_status(full, volume_tag),
                        element_address: desc.element_address,
                    };
                    result.transports.push(status);
                }
                2 | 3 => {
                    let desc: StorageDescriptor = unsafe { reader.read_be_value()? };

                    let full = (desc.flags1 & 1) != 0;
                    let volume_tag = subhead.parse_optional_volume_tag(&mut reader, full)?;

                    subhead.skip_alternate_volume_tag(&mut reader)?;

                    result.last_element_address = Some(desc.element_address);

                    if subhead.element_type_code == 3 {
                        let status = StorageElementStatus {
                            import_export: true,
                            status: create_element_status(full, volume_tag),
                            element_address: desc.element_address,
                        };
                        result.import_export_slots.push(status);
                    } else {
                        let status = StorageElementStatus {
                            import_export: false,
                            status: create_element_status(full, volume_tag),
                            element_address: desc.element_address,
                        };
                        result.storage_slots.push(status);
                    }
                }
                4 => {
                    let desc: TransferDescriptor = unsafe { reader.read_be_value()? };

                    let loaded_slot = if (desc.flags2 & 128) != 0 {
                        // SValid
                        Some(desc.source_storage_element_address as u64)
                    } else {
                        None
                    };

                    let full = (desc.flags1 & 1) != 0;
                    let volume_tag = subhead.parse_optional_volume_tag(&mut reader, full)?;

                    subhead.skip_alternate_volume_tag(&mut reader)?;

                    let dvcid = decode_dvcid_info(&mut reader).unwrap_or(DvcidInfo {
                        vendor: None,
                        model: None,
                        serial: None,
                    });

                    result.last_element_address = Some(desc.element_address);

                    let drive = DriveStatus {
                        loaded_slot,
                        status: create_element_status(full, volume_tag),
                        drive_serial_number: dvcid.serial,
                        vendor: dvcid.vendor,
                        model: dvcid.model,
                        element_address: desc.element_address,
                    };
                    result.drives.push(drive);
                }
                code => bail!("got unknown element type code {}", code),
            }
        }
    }

    Ok(result)
}

/// Open the device for read/write, returns the file handle
pub fn open<P: AsRef<Path>>(path: P) -> Result<File, Error> {
    let file = OpenOptions::new().read(true).write(true).open(path)?;

    Ok(file)
}

#[cfg(test)]
mod test {
    use super::*;
    use anyhow::Error;

    struct StorageDesc {
        address: u16,
        pvoltag: Option<String>,
    }

    fn build_element_status_page(
        descriptors: Vec<StorageDesc>,
        trailing: &[u8],
        element_type: u8,
    ) -> Vec<u8> {
        let descs: Vec<Vec<u8>> = descriptors
            .iter()
            .map(|desc| build_storage_descriptor(desc, trailing))
            .collect();

        let (desc_len, address) = if let Some(el) = descs.get(0) {
            (el.len() as u16, descriptors[0].address)
        } else {
            (0u16, 0u16)
        };

        let descriptor_byte_count = desc_len * descs.len() as u16;
        let byte_count = 8 + descriptor_byte_count;

        let mut res = Vec::new();

        res.extend_from_slice(&address.to_be_bytes());
        res.extend_from_slice(&(descs.len() as u16).to_be_bytes());
        res.push(0);
        let byte_count = byte_count as u32;
        res.extend_from_slice(&byte_count.to_be_bytes()[1..]);

        res.push(element_type);
        res.push(0x80);
        res.extend_from_slice(&desc_len.to_be_bytes());
        res.push(0);
        let descriptor_byte_count = descriptor_byte_count as u32;
        res.extend_from_slice(&descriptor_byte_count.to_be_bytes()[1..]);

        for desc in descs {
            res.extend_from_slice(&desc);
        }

        res.extend_from_slice(trailing);

        res
    }

    fn build_storage_descriptor(desc: &StorageDesc, trailing: &[u8]) -> Vec<u8> {
        let mut res = Vec::new();
        res.push(((desc.address >> 8) & 0xFF) as u8);
        res.push((desc.address & 0xFF) as u8);
        if desc.pvoltag.is_some() {
            res.push(0x01); // full
        } else {
            res.push(0x00); // full
        }

        res.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0x80]);
        res.push(((desc.address >> 8) & 0xFF) as u8);
        res.push((desc.address & 0xFF) as u8);

        if let Some(voltag) = &desc.pvoltag {
            res.extend_from_slice(voltag.as_bytes());
            let rem = SCSI_VOLUME_TAG_LEN - voltag.as_bytes().len();
            if rem > 0 {
                res.resize(res.len() + rem, 0);
            }
        }

        res.extend_from_slice(trailing);

        res
    }

    #[test]
    fn status_page_valid() -> Result<(), Error> {
        let descs = vec![
            StorageDesc {
                address: 0,
                pvoltag: Some("0123456789".to_string()),
            },
            StorageDesc {
                address: 1,
                pvoltag: Some("1234567890".to_string()),
            },
        ];
        let test_data = build_element_status_page(descs, &[], 0x2);
        let page = decode_element_status_page(&test_data, 0)?;
        assert_eq!(page.storage_slots.len(), 2);
        Ok(())
    }

    #[test]
    fn status_page_too_short() -> Result<(), Error> {
        let descs = vec![
            StorageDesc {
                address: 0,
                pvoltag: Some("0123456789".to_string()),
            },
            StorageDesc {
                address: 1,
                pvoltag: Some("1234567890".to_string()),
            },
        ];
        let test_data = build_element_status_page(descs, &[], 0x2);
        let len = test_data.len();
        let res = decode_element_status_page(&test_data[..(len - 10)], 0);
        assert!(res.is_err());
        Ok(())
    }

    #[test]
    fn status_page_too_large() -> Result<(), Error> {
        let descs = vec![
            StorageDesc {
                address: 0,
                pvoltag: Some("0123456789".to_string()),
            },
            StorageDesc {
                address: 1,
                pvoltag: Some("1234567890".to_string()),
            },
        ];
        let test_data = build_element_status_page(descs, &[0, 0, 0, 0, 0], 0x2);
        let page = decode_element_status_page(&test_data, 0)?;
        assert_eq!(page.storage_slots.len(), 2);
        Ok(())
    }
}
