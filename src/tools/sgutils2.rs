//! Bindings for libsgutils2
//!
//! Incomplete, but we currently do not need more.
//!
//! See: `/usr/include/scsi/sg_pt.h`

use std::os::unix::io::AsRawFd;
use std::ptr::NonNull;

use anyhow::{bail, format_err, Error};
use endian_trait::Endian;
use serde::{Deserialize, Serialize};
use libc::{c_char, c_int};

use proxmox::tools::io::ReadExt;

#[derive(Debug)]
pub struct SenseInfo {
    pub sense_key: u8,
    pub asc: u8,
    pub ascq: u8,
}

impl ToString for SenseInfo {

    fn to_string(&self) -> String {

        // Added codes from IBM TS4300 Tape Library SCSI reference manual
        // Added codes from Quantum Intelligent Libraries SCSI Reference Guide
        match (self.sense_key, self.asc, self.ascq) {
            // Not Ready
            (0x02, 0x04, 0x00) => String::from("Not ready, cause not reportable"),
            (0x02, 0x04, 0x01) => String::from("Not ready, operation in progress"),
            (0x02, 0x04, 0x03) => String::from("Not ready, manual intervention required"),
            (0x02, 0x04, 0x12) => String::from("Not ready, offline"),
            (0x02, 0x04, 0x83) => String::from("The library is not ready due to aisle power being disabled"),
            (0x02, 0x04, 0x8D) => String::from(" The library is not ready because it is offline"),
            (0x02, 0x3B, 0x12) => String::from("Not ready, magazine removed"),
            // Media Error
            (0x03, 0x30, 0x00) => String::from("Media error"),
            (0x03, 0x30, 0x07) => String::from("Cleaning failure"),
            // Hardware Error
            (0x04, 0x15, 0x01) => String::from("A mechanical positioning error occurred"),
            (0x04, 0x3B, 0x0D) => String::from("Medium destination element full"),
            (0x04, 0x3B, 0x0E) => String::from("Medium source element empty"),
            (0x04, 0x3F, 0x0F) => String::from("Echo buffer overwritten"),
            (0x04, 0x40, 0x80) => String::from("Component failure"),
            (0x04, 0x44, 0x00) => String::from("Firmware detected an internal logic failure"),
            (0x04, 0x53, 0x00) => String::from("A drive did not load or unload a tape"),
            (0x04, 0x53, 0x01) => String::from("A drive did not unload a cartridge"),
            (0x04, 0x53, 0x82) => String::from("Cannot lock the I/E station"),
            (0x04, 0x53, 0x83) => String::from("Cannot unlock the I/E station"),
            (0x04, 0x80, 0xD7) => String::from("Internal software error"),
            (0x04, 0x80, 0xD8) => String::from("Database access error"),
            (0x04, 0x81, 0xB0) => String::from("Internal system communication failed"),
            (0x04, 0x81, 0xB2) => String::from("Robotic controller communication failed"),
            (0x04, 0x81, 0xB3) => String::from("Mechanical positioning error"),
            (0x04, 0x81, 0xB4) => String::from("Cartridge did not transport completely."),
            (0x04, 0x82, 0xFC) => String::from("Drive configuration failed"),
            (0x04, 0x83, 0x00) => String::from("Label too short or too long"),
            // Illegal Request
            (0x05, 0x04, 0x83) => String::from("Door open"),
            (0x05, 0x1A, 0x00) => String::from("Parameter length error"),
            (0x05, 0x20, 0x00) => String::from("Invalid command operation code"),
            (0x05, 0x21, 0x01) => String::from("Invalid element address"),
            (0x05, 0x24, 0x00) => String::from("Invalid field CDB"),
            (0x05, 0x25, 0x00) => String::from("Invalid LUN"),
            (0x05, 0x26, 0x00) => String::from("Invalid field in parameter list"),
            (0x05, 0x26, 0x01) => String::from("Parameter list error: parameter not supported"),
            (0x05, 0x26, 0x02) => String::from("Parameter value invalid"),
            (0x05, 0x26, 0x04) => String::from("Invalid release of persistent reservation"),
            (0x05, 0x2C, 0x00) => String::from("Saving parameters is not supported"),
            (0x05, 0x30, 0x00) => String::from("Incompatible medium installed"),
            (0x05, 0x30, 0x12) => String::from("Incompatible Media loaded to drive"),
            (0x05, 0x39, 0x00) => String::from("Saving parameters is not supported"),
            (0x05, 0x3B, 0x0D) => String::from("Medium destination element full"),
            (0x05, 0x3B, 0x0E) => String::from("Medium source element empty"),
            (0x05, 0x3B, 0x11) => String::from("Magazine not accessible"),
            (0x05, 0x3B, 0x12) => String::from("Magazine not installed"),
            (0x05, 0x3B, 0x18) => String::from("Element disabled"),
            (0x05, 0x3B, 0x1A) => String::from("Data transfer element removed"),
            (0x05, 0x3B, 0xA0) => String::from("Media type does not match destination media type"),
            (0x05, 0x3F, 0x01) => String::from("New firmware loaded"),
            (0x05, 0x3F, 0x03) => String::from("Inquiry data changed"),
            (0x05, 0x44, 0x81) => String::from("Source element not ready"),
            (0x05, 0x44, 0x82) => String::from("Destination element not ready"),
            (0x05, 0x53, 0x02) => String::from("Library media removal prevented state set."),
            (0x05, 0x53, 0x03) => String::from("Drive media removal prevented state set"),
            (0x05, 0x53, 0x81) => String::from("Insert/Eject station door is open"),
            (0x05, 0x82, 0x93) => String::from("Failure session sequence error"),
            (0x05, 0x82, 0x94) => String::from("Failover command sequence error"),
            (0x05, 0x82, 0x95) => String::from("Duplicate failover session key"),
            (0x05, 0x82, 0x96) => String::from("Invalid failover key"),
            (0x05, 0x82, 0x97) => String::from("Failover session that is released"),
            (0x05, 0x83, 0x02) => String::from("Barcode label questionable"),
            (0x05, 0x83, 0x03) => String::from("Cell status and barcode label questionable"),
            (0x05, 0x83, 0x04) => String::from("Data transfer element not installed"),
            (0x05, 0x83, 0x05) => String::from("Data transfer element is varied off and not accessible for library operations"),
            (0x05, 0x83, 0x06) => String::from("Element is contained within an offline tower or I/E station and is not accessible for library operations"),
            // Unit Attention
            (0x06, 0x28, 0x00) => String::from("Not ready to change, medium changed"),
            (0x06, 0x28, 0x01) => String::from("Import/export element that is accessed"),
            (0x06, 0x29, 0x00) => String::from("Power-on or reset occurred"),
            (0x06, 0x29, 0x01) => String::from("Power on occurred"),
            (0x06, 0x29, 0x02) => String::from("SCSI Bus reset occurred"),
            (0x06, 0x29, 0x03) => String::from("Device reset occurred"),
            (0x06, 0x29, 0x04) => String::from("Internal reset occurred"),
            (0x06, 0x2A, 0x01) => String::from("Mode parameters have been changed"),
            (0x06, 0x2A, 0x03) => String::from("Reservations preempted"),
            (0x06, 0x2A, 0x04) => String::from("Reservations released"),
            (0x06, 0x2A, 0x05) => String::from("Registrations preempted"),
             // Aborted Command
            (0x0B, 0x3F, 0x0F) => String::from("ECHO buffer overwritten"),
            (0x0B, 0x44, 0x00) => String::from("Firmware detected an internal logic failure"),
            (0x0B, 0x45, 0x00) => String::from("Select or reselect failure"),
            (0x0B, 0x47, 0x00) => String::from("SCSI parity error"),
            (0x0B, 0x48, 0x00) => String::from("Initiator detected error message received"),
            (0x0B, 0x49, 0x00) => String::from("Invalid message error"),
            (0x0B, 0x4A, 0x00) => String::from("Command phase error"),
            (0x0B, 0x4B, 0x00) => String::from("Data phase error"),
            (0x0B, 0x4E, 0x00) => String::from("Overlapped command attempt"),
            // Else, simply report values
            _ => format!("sense_key = 0x{:02x}, ASC = 0x{:02x}, ASCQ = 0x{:02x}", self.sense_key, self.asc, self.ascq),
        }
    }
}

#[derive(Debug)]
pub struct ScsiError {
    pub error: Error,
    pub sense: Option<SenseInfo>,
}

impl std::fmt::Display for ScsiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.error)
    }
}

impl std::error::Error for ScsiError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.error.source()
    }
}

impl From<anyhow::Error> for ScsiError {
    fn from(error: anyhow::Error) -> Self {
        Self { error, sense: None }
    }
}

impl From<std::io::Error> for ScsiError {
    fn from(error: std::io::Error) -> Self {
        Self { error: error.into(), sense: None }
    }
}

// Opaque wrapper for sg_pt_base
#[repr(C)]
struct SgPtBase { _private: [u8; 0] }

#[repr(transparent)]
struct SgPt {
    raw: NonNull<SgPtBase>,
}

impl Drop for SgPt {
    fn drop(&mut self) {
        unsafe { destruct_scsi_pt_obj(self.as_mut_ptr()) };
    }
}

impl SgPt {
    fn new() -> Result<Self, Error> {
        Ok(Self {
            raw: NonNull::new(unsafe { construct_scsi_pt_obj() })
                .ok_or_else(|| format_err!("construct_scsi_pt_ob failed"))?,
        })
    }

    fn as_ptr(&self) -> *const SgPtBase {
        self.raw.as_ptr()
    }

    fn as_mut_ptr(&mut self) -> *mut SgPtBase {
        self.raw.as_ptr()
    }
}

/// Peripheral device type text (see `inquiry` command)
///
/// see [https://en.wikipedia.org/wiki/SCSI_Peripheral_Device_Type]
pub const PERIPHERAL_DEVICE_TYPE_TEXT: [&'static str; 32] = [
    "Disk Drive",
    "Tape Drive",
    "Printer",
    "Processor",
    "Write-once",
    "CD-ROM",  // 05h
    "Scanner",
    "Optical",
    "Medium Changer", // 08h
    "Communications",
    "ASC IT8",
    "ASC IT8",
    "RAID Array",
    "Enclosure Services",
    "Simplified direct-access",
    "Optical card reader/writer",
    "Bridging Expander",
    "Object-based Storage",
    "Automation/Drive Interface",
    "Security manager",
    "Reserved",
    "Reserved",
    "Reserved",
    "Reserved",
    "Reserved",
    "Reserved",
    "Reserved",
    "Reserved",
    "Reserved",
    "Reserved",
    "Reserved",
    "Unknown",
];

//  SENSE KEYS
pub const SENSE_KEY_NO_SENSE: u8        = 0x00;
pub const SENSE_KEY_RECOVERED_ERROR: u8 = 0x01;
pub const SENSE_KEY_NOT_READY: u8       = 0x02;
pub const SENSE_KEY_MEDIUM_ERROR: u8    = 0x03;
pub const SENSE_KEY_HARDWARE_ERROR: u8  = 0x04;
pub const SENSE_KEY_ILLEGAL_REQUEST: u8 = 0x05;
pub const SENSE_KEY_UNIT_ATTENTION: u8  = 0x06;
pub const SENSE_KEY_DATA_PROTECT: u8    = 0x07;
pub const SENSE_KEY_BLANK_CHECK: u8     = 0x08;
pub const SENSE_KEY_COPY_ABORTED: u8    = 0x0a;
pub const SENSE_KEY_ABORTED_COMMAND: u8 = 0x0b;
pub const SENSE_KEY_VOLUME_OVERFLOW: u8 = 0x0d;
pub const SENSE_KEY_MISCOMPARE: u8      = 0x0e;


#[repr(C, packed)]
#[derive(Endian)]
struct InquiryPage {
    peripheral_type: u8,
    rmb: u8,
    version: u8,
    flags3: u8,
    additional_length: u8,
    flags5: u8,
    flags6: u8,
    flags7: u8,
    vendor: [u8; 8],
    product: [u8; 16],
    revision: [u8; 4],
    // additional data follows, but we do not need that
}

#[repr(C, packed)]
#[derive(Endian, Debug)]
struct RequestSensePage {
    response_code: u8,
    obsolete: u8,
    flags2: u8,
    information: [u8;4],
    additional_sense_len: u8,
    command_specific_information: [u8;4],
    additional_sense_code: u8,
    additional_sense_code_qualifier: u8,
    field_replacable_unit_code: u8,
    sense_key_specific: [u8; 3],
}

/// Inquiry result
#[derive(Serialize, Deserialize, Debug)]
pub struct InquiryInfo {
    /// Peripheral device type (0-31)
    pub peripheral_type: u8,
    /// Peripheral device type as string
    pub peripheral_type_text: String,
    /// Vendor
    pub vendor: String,
    /// Product
    pub product: String,
    /// Revision
    pub revision: String,
}

pub const SCSI_PT_DO_START_OK:c_int = 0;
pub const SCSI_PT_DO_BAD_PARAMS:c_int = 1;
pub const SCSI_PT_DO_TIMEOUT:c_int = 2;

pub const SCSI_PT_RESULT_GOOD:c_int = 0;
pub const SCSI_PT_RESULT_STATUS:c_int = 1;
pub const SCSI_PT_RESULT_SENSE:c_int = 2;
pub const SCSI_PT_RESULT_TRANSPORT_ERR:c_int = 3;
pub const SCSI_PT_RESULT_OS_ERR:c_int = 4;

#[link(name = "sgutils2")]
extern "C" {

    #[allow(dead_code)]
    fn scsi_pt_open_device(
        device_name: * const c_char,
        read_only: bool,
        verbose: c_int,
    ) -> c_int;

    fn sg_is_scsi_cdb(
        cdbp: *const u8,
        clen: c_int,
    ) -> bool;

    fn construct_scsi_pt_obj() -> *mut SgPtBase;
    fn destruct_scsi_pt_obj(objp: *mut SgPtBase);

    fn set_scsi_pt_data_in(
        objp: *mut SgPtBase,
        dxferp: *mut u8,
        dxfer_ilen: c_int,
    );

    fn set_scsi_pt_data_out(
        objp: *mut SgPtBase,
        dxferp: *const u8,
        dxfer_olen: c_int,
    );

    fn set_scsi_pt_cdb(
        objp: *mut SgPtBase,
        cdb: *const u8,
        cdb_len: c_int,
    );

    fn set_scsi_pt_sense(
        objp: *mut SgPtBase,
        sense: *mut u8,
        max_sense_len: c_int,
    );

    fn do_scsi_pt(
        objp: *mut SgPtBase,
        fd: c_int,
        timeout_secs: c_int,
        verbose: c_int,
    ) -> c_int;

    fn get_scsi_pt_resid(objp: *const SgPtBase) -> c_int;

    fn get_scsi_pt_sense_len(objp: *const SgPtBase) -> c_int;

    fn get_scsi_pt_status_response(objp: *const SgPtBase) -> c_int;

    fn get_scsi_pt_result_category(objp: *const SgPtBase) -> c_int;

    fn get_scsi_pt_os_err(objp: *const SgPtBase) -> c_int;
}

/// Safe interface to run RAW SCSI commands
pub struct SgRaw<'a, F> {
    file: &'a mut F,
    buffer: Box<[u8]>,
    sense_buffer: [u8; 32],
    timeout: i32,
}

/// Allocate a page aligned buffer
///
/// SG RAWIO commands needs page aligned transfer buffers.
pub fn alloc_page_aligned_buffer(buffer_size: usize) -> Result<Box<[u8]> , Error> {
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) } as usize;
    let layout = std::alloc::Layout::from_size_align(buffer_size, page_size)?;
    let dinp = unsafe { std::alloc::alloc_zeroed(layout) };
    if dinp.is_null() {
        bail!("alloc SCSI output buffer failed");
    }

    let buffer_slice = unsafe { std::slice::from_raw_parts_mut(dinp, buffer_size)};
    Ok(unsafe { Box::from_raw(buffer_slice) })
}

impl <'a, F: AsRawFd> SgRaw<'a, F> {

    /// Create a new instance to run commands
    ///
    /// The file must be a handle to a SCSI device.
    pub fn new(file: &'a mut F, buffer_size: usize) -> Result<Self, Error> {

        let buffer;

        if buffer_size > 0 {
            buffer = alloc_page_aligned_buffer(buffer_size)?;
        } else {
            buffer =  Box::new([]);
        }

        let sense_buffer = [0u8; 32];

        Ok(Self { file, buffer, sense_buffer, timeout: 0 })
    }

    /// Set the command timeout in seconds (0 means default (60 seconds))
    pub fn set_timeout(&mut self, seconds: usize) {
        if seconds > (i32::MAX as usize) {
            self.timeout = i32::MAX; // don't care about larger values
        } else {
            self.timeout = seconds as i32;
        }
    }

    // create new object with initialized data_in and sense buffer
    fn create_scsi_pt_obj(&mut self) -> Result<SgPt, Error> {

        let mut ptvp = SgPt::new()?;

        if self.buffer.len() > 0 {
            unsafe {
                set_scsi_pt_data_in(
                    ptvp.as_mut_ptr(),
                    self.buffer.as_mut_ptr(),
                    self.buffer.len() as c_int,
                )
            };
        }

        unsafe {
            set_scsi_pt_sense(
                ptvp.as_mut_ptr(),
                self.sense_buffer.as_mut_ptr(),
                self.sense_buffer.len() as c_int,
            )
        };

        Ok(ptvp)
    }

    fn do_scsi_pt_checked(&mut self, ptvp: &mut SgPt) -> Result<(), ScsiError> {

        let res = unsafe { do_scsi_pt(ptvp.as_mut_ptr(), self.file.as_raw_fd(), self.timeout, 0) };
        match res {
            SCSI_PT_DO_START_OK => { /* Ok */ },
            SCSI_PT_DO_BAD_PARAMS => return Err(format_err!("do_scsi_pt failed - bad pass through setup").into()),
            SCSI_PT_DO_TIMEOUT => return Err(format_err!("do_scsi_pt failed - timeout").into()),
            code if code < 0 => {
                let errno = unsafe { get_scsi_pt_os_err(ptvp.as_ptr()) };
                let err = nix::Error::from_errno(nix::errno::Errno::from_i32(errno));
                return Err(format_err!("do_scsi_pt failed with err {}", err).into());
            }
            unknown => return Err(format_err!("do_scsi_pt failed: unknown error {}", unknown).into()),
        }

        if res < 0 {
            let err = nix::Error::last();
            return Err(format_err!("do_scsi_pt failed  - {}", err).into());
        }
        if res != 0 {
            return Err(format_err!("do_scsi_pt failed {}", res).into());
        }

        let sense_len = unsafe { get_scsi_pt_sense_len(ptvp.as_ptr()) };

        let res_cat = unsafe { get_scsi_pt_result_category(ptvp.as_ptr()) };
        match res_cat {
            SCSI_PT_RESULT_GOOD => { /* Ok */ }
            SCSI_PT_RESULT_STATUS => { /* test below */ }
            SCSI_PT_RESULT_SENSE => {
                if sense_len == 0 {
                    return Err(format_err!("scsi command failed: no Sense").into());
                }
                let mut reader = &self.sense_buffer[..(sense_len as usize)];

                let sense: RequestSensePage = unsafe { reader.read_be_value()? };

                let code = sense.response_code & 0x7f;
                if code == 0x71 {
                    return Err(format_err!("scsi command failed: received deferred Sense").into());
                }
                if code != 0x70 {
                    return Err(format_err!("scsi command failed: invalid Sense response code {:x}", code).into());
                }

                let sense = SenseInfo {
                    sense_key: sense.flags2 & 0xf,
                    asc: sense.additional_sense_code,
                    ascq: sense.additional_sense_code_qualifier,
                };

                return Err(ScsiError {
                    error: format_err!("scsi command failed: {}", sense.to_string()),
                    sense: Some(sense),
                });
            }
            SCSI_PT_RESULT_TRANSPORT_ERR => return Err(format_err!("scsi command failed: transport error").into()),
            SCSI_PT_RESULT_OS_ERR => {
                let errno = unsafe { get_scsi_pt_os_err(ptvp.as_ptr()) };
                let err = nix::Error::from_errno(nix::errno::Errno::from_i32(errno));
                return Err(format_err!("scsi command failed with err {}", err).into());
            }
            unknown => return Err(format_err!("scsi command failed: unknown result category {}", unknown).into()),
        }

        let status = unsafe { get_scsi_pt_status_response(ptvp.as_ptr()) };
        if status != 0 {
            return Err(format_err!("unknown scsi error - status response {}", status).into());
        }

        Ok(())
    }

    /// Run the specified RAW SCSI command
    pub fn do_command(&mut self, cmd: &[u8]) -> Result<&[u8], ScsiError> {

        if !unsafe { sg_is_scsi_cdb(cmd.as_ptr(), cmd.len() as c_int) } {
            return Err(format_err!("no valid SCSI command").into());
        }

        if self.buffer.len() < 16 {
            return Err(format_err!("output buffer too small").into());
        }

        let mut ptvp = self.create_scsi_pt_obj()?;

        unsafe {
            set_scsi_pt_cdb(
                ptvp.as_mut_ptr(),
                cmd.as_ptr(),
                cmd.len() as c_int,
            )
        };

        self.do_scsi_pt_checked(&mut ptvp)?;

        let resid = unsafe { get_scsi_pt_resid(ptvp.as_ptr()) } as usize;
        if resid > self.buffer.len() {
            return Err(format_err!("do_scsi_pt failed - got strange resid (value too big)").into());
        }
        let data_len = self.buffer.len() - resid;

        Ok(&self.buffer[..data_len])
    }

    /// Run dataout command
    ///
    /// Note: use alloc_page_aligned_buffer to alloc data transfer buffer
    pub fn do_out_command(&mut self, cmd: &[u8], data: &[u8]) -> Result<(), Error> {

        if !unsafe { sg_is_scsi_cdb(cmd.as_ptr(), cmd.len() as c_int) } {
            bail!("no valid SCSI command");
        }

        let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) } as usize;
        if ((data.as_ptr() as usize) & (page_size -1)) != 0 {
            bail!("wrong transfer buffer alignment");
        }

        let mut ptvp = self.create_scsi_pt_obj()?;

        unsafe {
            set_scsi_pt_data_out(
                ptvp.as_mut_ptr(),
                data.as_ptr(),
                data.len() as c_int,
            );

            set_scsi_pt_cdb(
                ptvp.as_mut_ptr(),
                cmd.as_ptr(),
                cmd.len() as c_int,
            );
        };

        self.do_scsi_pt_checked(&mut ptvp)?;

        Ok(())
    }
}

// Useful helpers

/// Converts SCSI ASCII text into String, trim zero and spaces
pub fn scsi_ascii_to_string(data: &[u8]) -> String {
    String::from_utf8_lossy(data)
        .trim_matches(char::from(0))
        .trim()
        .to_string()
}

/// Read SCSI Inquiry page
///
/// Returns Product/Vendor/Revision and device type.
pub fn scsi_inquiry<F: AsRawFd>(
    file: &mut F,
) -> Result<InquiryInfo, Error> {

    let allocation_len: u8 = u8::MAX;
    let mut sg_raw = SgRaw::new(file, allocation_len as usize)?;
    sg_raw.set_timeout(30); // use short timeout

    let mut cmd = Vec::new();
    cmd.extend(&[0x12, 0, 0, 0, allocation_len, 0]); // INQUIRY

    let data = sg_raw.do_command(&cmd)
        .map_err(|err| format_err!("SCSI inquiry failed - {}", err))?;

    proxmox::try_block!({
        let mut reader = &data[..];

        let page: InquiryPage  = unsafe { reader.read_be_value()? };

        let peripheral_type = page.peripheral_type & 31;

        let info = InquiryInfo {
            peripheral_type,
            peripheral_type_text: PERIPHERAL_DEVICE_TYPE_TEXT[peripheral_type as usize].to_string(),
            vendor: scsi_ascii_to_string(&page.vendor),
            product: scsi_ascii_to_string(&page.product),
            revision: scsi_ascii_to_string(&page.revision),
        };

        Ok(info)
    }).map_err(|err: Error| format_err!("decode inquiry page failed - {}", err))
}
