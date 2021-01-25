//! Bindings for libsgutils2
//!
//! Incomplete, but we currently do not need more.
//!
//! See: `/usr/include/scsi/sg_pt.h`

use std::os::unix::io::AsRawFd;

use anyhow::{bail, format_err, Error};
use libc::{c_char, c_int};
use endian_trait::Endian;

use proxmox::tools::io::ReadExt;

// Opaque wrapper for sg_pt_base
#[repr(C)]
struct SgPtBase { _private: [u8; 0] }

impl Drop for SgPtBase  {
    fn drop(&mut self) {
        unsafe { destruct_scsi_pt_obj(self as *mut SgPtBase) };
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

/// Inquiry result
#[derive(Debug)]
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

pub const SCSI_PT_RESULT_GOOD:c_int = 0;
pub const SCSI_PT_RESULT_STATUS:c_int = 1;
pub const SCSI_PT_RESULT_SENSE:c_int = 2;
pub const SCSI_PT_RESULT_TRANSPORT_ERR:c_int = 3;
pub const SCSI_PT_RESULT_OS_ERR:c_int = 4;

#[link(name = "sgutils2")]
extern {

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

    #[allow(dead_code)]
    fn get_scsi_pt_result_category(objp: *const SgPtBase) -> c_int;
}

/// Creates a `Box<SgPtBase>`
///
/// Which get automatically dropped, so you do not need to call
/// destruct_scsi_pt_obj yourself.
fn boxed_scsi_pt_obj() -> Result<Box<SgPtBase>, Error> {
    let objp = unsafe {
        construct_scsi_pt_obj()
    };
    if objp.is_null() {
        bail!("construct_scsi_pt_ob failed");
    }

    Ok(unsafe { std::mem::transmute(objp)})
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
    fn create_boxed_scsi_pt_obj(&mut self) -> Result<Box<SgPtBase>, Error> {

        let mut ptvp = boxed_scsi_pt_obj()?;

        if self.buffer.len() > 0 {
            unsafe {
                set_scsi_pt_data_in(
                    &mut *ptvp,
                    self.buffer.as_mut_ptr(),
                    self.buffer.len() as c_int,
                )
            };
        }

        unsafe {
            set_scsi_pt_sense(
                &mut *ptvp,
                self.sense_buffer.as_mut_ptr(),
                self.sense_buffer.len() as c_int,
            )
        };

        Ok(ptvp)
    }

    /// Run the specified RAW SCSI command
    pub fn do_command(&mut self, cmd: &[u8]) -> Result<&[u8], Error> {

        if !unsafe { sg_is_scsi_cdb(cmd.as_ptr(), cmd.len() as c_int) } {
            bail!("no valid SCSI command");
        }

        if self.buffer.len() < 16 {
            bail!("output buffer too small");
        }

        let mut ptvp = self.create_boxed_scsi_pt_obj()?;

        unsafe {
            set_scsi_pt_cdb(
                &mut *ptvp,
                cmd.as_ptr(),
                cmd.len() as c_int,
            )
        };

        let res = unsafe { do_scsi_pt(&mut *ptvp, self.file.as_raw_fd(), self.timeout, 0) };
        if res < 0 {
            let err = nix::Error::last();
            bail!("do_scsi_pt failed  - {}", err);
        }
        if res != 0 {
            bail!("do_scsi_pt failed {}", res);
        }

        // todo: what about sense data?
        let _sense_len = unsafe { get_scsi_pt_sense_len(&*ptvp) };

        let status = unsafe { get_scsi_pt_status_response(&*ptvp) };
        if status != 0 {
            // toto: improve error reporting
            bail!("unknown scsi error - status response {}", status);
        }

        let resid = unsafe { get_scsi_pt_resid(&*ptvp) } as usize;
        if resid > self.buffer.len() {
            bail!("do_scsi_pt failed - got strange resid (value too big)");
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

        let mut ptvp = self.create_boxed_scsi_pt_obj()?;

        unsafe {
            set_scsi_pt_data_out(
                &mut *ptvp,
                data.as_ptr(),
                data.len() as c_int,
            );

            set_scsi_pt_cdb(
                &mut *ptvp,
                cmd.as_ptr(),
                cmd.len() as c_int,
            );
         };

        let res = unsafe { do_scsi_pt(&mut *ptvp, self.file.as_raw_fd(), self.timeout, 0) };
        if res < 0 {
            let err = nix::Error::last();
            bail!("do_scsi_pt failed  - {}", err);
        }
        if res != 0 {
            bail!("do_scsi_pt failed {}", res);
        }

        // todo: what about sense data?
        let _sense_len = unsafe { get_scsi_pt_sense_len(&*ptvp) };

        let status = unsafe { get_scsi_pt_status_response(&*ptvp) };
        if status != 0 {
            // toto: improve error reporting
            bail!("unknown scsi error - status response {}", status);
        }

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
