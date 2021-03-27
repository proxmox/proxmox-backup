//! SCSI changer implementation using libsgutil2

use std::os::unix::prelude::AsRawFd;
use std::io::Read;
use std::collections::HashMap;
use std::path::Path;
use std::fs::{OpenOptions, File};

use anyhow::{bail, format_err, Error};
use endian_trait::Endian;

use proxmox::tools::io::ReadExt;

use crate::{
    tape::{
        changer::{
            DriveStatus,
            ElementStatus,
            StorageElementStatus,
            TransportElementStatus,
            MtxStatus,
        },
    },
    tools::sgutils2::{
        SgRaw,
        SENSE_KEY_NO_SENSE,
        SENSE_KEY_RECOVERED_ERROR,
        SENSE_KEY_UNIT_ATTENTION,
        SENSE_KEY_NOT_READY,
        InquiryInfo,
        ScsiError,
        scsi_ascii_to_string,
        scsi_inquiry,
    },
    api2::types::ScsiTapeChanger,
};

const SCSI_CHANGER_DEFAULT_TIMEOUT: usize = 60*5; // 5 minutes

/// Initialize element status (Inventory)
pub fn initialize_element_status<F: AsRawFd>(file: &mut F) -> Result<(), Error> {

    let mut sg_raw = SgRaw::new(file, 64)?;

    // like mtx(1), set a very long timeout (30 minutes)
    sg_raw.set_timeout(30*60);

    let mut cmd = Vec::new();
    cmd.extend(&[0x07, 0, 0, 0, 0, 0]); // INITIALIZE ELEMENT STATUS (07h)

    sg_raw.do_command(&cmd)
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
/// successful (sleep 1 second between invovations)
///
/// Any Sense key other than NO_SENSE, RECOVERED_ERROR, NOT_READY and
/// UNIT_ATTENTION aborts the loop and returns an error. If the device
/// reports "Not Ready - becoming ready", we wait up to 5 minutes.
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
        match sg_raw.do_command(&cmd) {
            Ok(data) => return Ok(data.to_vec()),
            Err(err) => {
                if !retry {
                    bail!("{} failed: {}", error_prefix, err);
                }
                if let ScsiError::Sense(ref sense) = err {

                    if sense.sense_key == SENSE_KEY_NO_SENSE ||
                        sense.sense_key == SENSE_KEY_RECOVERED_ERROR ||
                        sense.sense_key == SENSE_KEY_UNIT_ATTENTION ||
                        sense.sense_key == SENSE_KEY_NOT_READY
                    {
                        let msg = err.to_string();
                        if let Some(ref last) = last_msg {
                            if &msg != last {
                                eprintln!("{}", err);
                                last_msg = Some(msg);
                            }
                        } else {
                            eprintln!("{}", err);
                            last_msg = Some(msg);
                        }

                        // Not Ready - becoming ready
                        if sense.sense_key == SENSE_KEY_NOT_READY && sense.asc == 0x04 && sense.ascq == 1 {
                            // wait up to 5 minutes, long enough to finish inventorize
                            timeout = std::time::Duration::new(5*60, 0);
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
   }
}


fn read_element_address_assignment<F: AsRawFd>(
    file: &mut F,
) -> Result<AddressAssignmentPage, Error> {

    let allocation_len: u8 = u8::MAX;
    let mut sg_raw = SgRaw::new(file, allocation_len as usize)?;
    sg_raw.set_timeout(SCSI_CHANGER_DEFAULT_TIMEOUT);

    let mut cmd = Vec::new();
    cmd.push(0x1A); // MODE SENSE6 (1Ah)
    cmd.push(0x08); // DBD=1 (The Disable Block Descriptors)
    cmd.push(0x1D); // Element Address Assignment Page
    cmd.push(0);
    cmd.push(allocation_len); // allocation len
    cmd.push(0); //control

    let data = execute_scsi_command(&mut sg_raw, &cmd, "read element address assignment", true)?;

    proxmox::try_block!({
        let mut reader = &data[..];
        let page: AddressAssignmentPage = unsafe { reader.read_be_value()? };

        if page.data_len != 23 {
            bail!("got unexpected page len ({} != 23)", page.data_len);
        }

        Ok(page)
    }).map_err(|err: Error| format_err!("decode element address assignment page failed - {}", err))
}

fn scsi_move_medium_cdb(
    medium_transport_address: u16,
    source_element_address: u16,
    destination_element_address: u16,
) -> Vec<u8> {

    let mut cmd = Vec::new();
    cmd.push(0xA5); // MOVE MEDIUM (A5h)
    cmd.push(0); // reserved
    cmd.extend(&medium_transport_address.to_be_bytes());
    cmd.extend(&source_element_address.to_be_bytes());
    cmd.extend(&destination_element_address.to_be_bytes());
    cmd.push(0); // reserved
    cmd.push(0); // reserved
    cmd.push(0); // Invert=0
    cmd.push(0); // control

    cmd
}

/// Load media from storage slot into drive
pub fn load_slot(
    file: &mut File,
    from_slot: u64,
    drivenum: u64,
) -> Result<(), Error> {
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

    sg_raw.do_command(&cmd)
        .map_err(|err| format_err!("load drive failed - {}", err))?;

    Ok(())
}

/// Unload media from drive into a storage slot
pub fn unload(
    file: &mut File,
    to_slot: u64,
    drivenum: u64,
) -> Result<(), Error> {

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

    sg_raw.do_command(&cmd)
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

    sg_raw.do_command(&cmd)
        .map_err(|err| {
            format_err!("transfer medium from slot {} to slot {} failed - {}",
                        from_slot, to_slot, err)
        })?;

    Ok(())
}

fn scsi_read_element_status_cdb(
    start_element_address: u16,
    allocation_len: u32,
) -> Vec<u8> {

    let mut cmd = Vec::new();
    cmd.push(0xB8); // READ ELEMENT STATUS (B8h)
    cmd.push(1u8<<4); // report all types and volume tags
    cmd.extend(&start_element_address.to_be_bytes());

    let number_of_elements: u16 = 0xffff;
    cmd.extend(&number_of_elements.to_be_bytes());
    cmd.push(0b001); //  Mixed=0,CurData=0,DVCID=1
    cmd.extend(&allocation_len.to_be_bytes()[1..4]);
    cmd.push(0);
    cmd.push(0);

    cmd
}

/// Read element status.
pub fn read_element_status<F: AsRawFd>(file: &mut F) -> Result<MtxStatus, Error> {

    let inquiry = scsi_inquiry(file)?;

    if inquiry.peripheral_type != 8 {
        bail!("wrong device type (not a scsi changer device)");
    }

    // first, request address assignment (used for sanity checks)
    let setup = read_element_address_assignment(file)?;

    let allocation_len: u32 = 0x10000;

    let mut sg_raw = SgRaw::new(file, allocation_len as usize)?;
    sg_raw.set_timeout(SCSI_CHANGER_DEFAULT_TIMEOUT);

    let mut start_element_address = 0;

    let mut drives = Vec::new();
    let mut storage_slots = Vec::new();
    let mut import_export_slots = Vec::new();
    let mut transports = Vec::new();

    let mut retry = true;

    loop {
        let cmd = scsi_read_element_status_cdb(start_element_address, allocation_len);

        let data = execute_scsi_command(&mut sg_raw, &cmd, "read element status (B8h)", retry)?;

        let page = decode_element_status_page(&inquiry, &data, start_element_address)?;

        retry = false; // only retry the first command

        transports.extend(page.transports);
        drives.extend(page.drives);
        storage_slots.extend(page.storage_slots);
        import_export_slots.extend(page.import_export_slots);

        if data.len() < (allocation_len as usize) {
            break;
        }

        if let Some(last_element_address) = page.last_element_address {
            if last_element_address >= start_element_address {
                start_element_address = last_element_address + 1;
            } else {
                bail!("got strange element address");
            }
        } else {
            break;
        }
    }

    if (setup.transport_element_count as usize) != transports.len() {
        bail!("got wrong number of transport elements");
    }
    if (setup.storage_element_count as usize) != storage_slots.len() {
        bail!("got wrong number of storage elements");
    }
    if (setup.import_export_element_count as usize) != import_export_slots.len() {
        bail!("got wrong number of import/export elements");
    }
    if (setup.transfer_element_count as usize) != drives.len() {
        bail!("got wrong number of transfer elements");
    }

    // create same virtual slot order as mtx(1)
    // - storage slots first
    // - import export slots at the end
    let mut slots = storage_slots;
    slots.extend(import_export_slots);

    let mut status = MtxStatus { transports, drives, slots };

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
            drive.loaded_slot = slot_map.get(&source_address).map(|v| *v);
        }
    }

    Ok(status)
}

/// Read status and map import-export slots from config
pub fn status(config: &ScsiTapeChanger) -> Result<MtxStatus, Error> {
    let path = &config.path;

    let mut file = open(path)
        .map_err(|err| format_err!("error opening '{}': {}", path, err))?;
    let mut status = read_element_status(&mut file)
        .map_err(|err| format_err!("error reading element status: {}", err))?;

    status.mark_import_export_slots(&config)?;

    Ok(status)
}


#[repr(C, packed)]
#[derive(Endian)]
struct ElementStatusHeader {
    first_element_address_reported: u16,
    number_of_elements_available: u16,
    reserved: u8,
    byte_count_of_report_available: [u8;3],
}

#[repr(C, packed)]
#[derive(Endian)]
struct SubHeader {
    element_type_code: u8,
    flags: u8,
    descriptor_length: u16,
    reserved: u8,
    byte_count_of_descriptor_data_available: [u8;3],
}

impl SubHeader {

    fn parse_optional_volume_tag<R: Read>(
        &self,
        reader: &mut R,
        full: bool,
    ) -> Result<Option<String>, Error> {

        if (self.flags & 128) != 0 { // has PVolTag
            let tmp = reader.read_exact_allocated(36)?;
            if full {
                let volume_tag = scsi_ascii_to_string(&tmp);
                return Ok(Some(volume_tag));
            }
        }
        Ok(None)
    }

    // AFAIK, tape changer do not use AlternateVolumeTag
    // but parse anyways, just to be sure
    fn skip_alternate_volume_tag<R: Read>(
        &self,
        reader: &mut R,
    ) -> Result<Option<String>, Error> {

        if (self.flags & 64) != 0 { // has AVolTag
            let _tmp = reader.read_exact_allocated(36)?;
        }

        Ok(None)
    }
}

#[repr(C, packed)]
#[derive(Endian)]
struct TrasnsportDescriptor { // Robot/Griper
    element_address: u16,
    flags1: u8,
    reserved_3: u8,
    additional_sense_code: u8,
    additional_sense_code_qualifier: u8,
    reserved_6: [u8;3],
    flags2: u8,
    source_storage_element_address: u16,
    // volume tag and Mixed media descriptor follows (depends on flags)
}

#[repr(C, packed)]
#[derive(Endian)]
struct TransferDescriptor { // Tape drive
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
struct DvcidHead { // Drive Identifier Header
    code_set: u8,
    identifier_type: u8,
    reserved: u8,
    identifier_len: u8,
    // Identifier follows
}

#[repr(C, packed)]
#[derive(Endian)]
struct StorageDescriptor { // Mail Slot
    element_address: u16,
    flags1: u8,
    reserved_3: u8,
    additional_sense_code: u8,
    additional_sense_code_qualifier: u8,
    reserved_6: [u8;3],
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

fn decode_element_status_page(
    _info: &InquiryInfo,
    data: &[u8],
    start_element_address: u16,
) -> Result<DecodedStatusPage, Error> {

    proxmox::try_block!({

        let mut result = DecodedStatusPage {
            last_element_address: None,
            transports: Vec::new(),
            drives: Vec::new(),
            storage_slots: Vec::new(),
            import_export_slots: Vec::new(),
        };

        let mut reader = &data[..];

        let head: ElementStatusHeader = unsafe { reader.read_be_value()? };

        if head.number_of_elements_available == 0 {
            return Ok(result);
        }

        if head.first_element_address_reported < start_element_address {
            bail!("got wrong first_element_address_reported"); // sanity check
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
            let mut reader = &descr_data[..];

            loop {
                if reader.is_empty() {
                    break;
                }
                if reader.len() < (subhead.descriptor_length as usize) {
                    break;
                }

                match subhead.element_type_code {
                    1 => {
                        let desc: TrasnsportDescriptor = unsafe { reader.read_be_value()? };

                        let full = (desc.flags1 & 1) != 0;
                        let volume_tag = subhead.parse_optional_volume_tag(&mut reader, full)?;

                        subhead.skip_alternate_volume_tag(&mut reader)?;

                        let mut reserved = [0u8; 4];
                        reader.read_exact(&mut reserved)?;

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

                        let mut reserved = [0u8; 4];
                        reader.read_exact(&mut reserved)?;

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

                        let loaded_slot = if (desc.flags2 & 128) != 0 { // SValid
                            Some(desc.source_storage_element_address as u64)
                        } else {
                            None
                        };

                        let full = (desc.flags1 & 1) != 0;
                        let volume_tag = subhead.parse_optional_volume_tag(&mut reader, full)?;

                        subhead.skip_alternate_volume_tag(&mut reader)?;

                        let dvcid: DvcidHead = unsafe { reader.read_be_value()? };

                        let (drive_serial_number, vendor, model) = match (dvcid.code_set, dvcid.identifier_type) {
                            (2, 0) => { // Serial number only (Quantum Superloader3 uses this)
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

                        result.last_element_address = Some(desc.element_address);

                        let drive = DriveStatus {
                            loaded_slot,
                            status: create_element_status(full, volume_tag),
                            drive_serial_number,
                            vendor,
                            model,
                            element_address: desc.element_address,
                        };
                        result.drives.push(drive);
                    }
                    code => bail!("got unknown element type code {}", code),
                }
            }
        }

        Ok(result)
    }).map_err(|err: Error| format_err!("decode element status failed - {}", err))
}

/// Open the device for read/write, returns the file handle
pub fn open<P: AsRef<Path>>(path: P) -> Result<File, Error> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)?;

    Ok(file)
}
