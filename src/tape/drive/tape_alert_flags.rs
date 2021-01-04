use std::io::Read;
use std::os::unix::io::AsRawFd;

use anyhow::{bail, format_err, Error};

use proxmox::tools::io::ReadExt;

use crate::{
    tape::{
        sgutils2::SgRaw,
    },
};

bitflags::bitflags!{

    /// Tape Alert Flags
    ///
    /// See LTO SCSI Reference LOG_SENSE - LP 2Eh: TapeAlerts
    pub struct TapeAlertFlags: u64 {
        const READ_WARNING = 1 << 0x0001;
        const WRITE_WARNING = 1 << 0002;
        const HARD_ERROR = 1 << 0x0003;
        const MEDIA = 1 << 0x0004;
        const READ_FAILURE = 1 << 0x0005;
        const WRITE_FAILURE = 1 << 0x0006;
        const MEDIA_LIFE = 1 << 0x0007;
        const NOT_DATA_GRADE = 1 << 0x0008;
        const WRITE_PROTECT = 1 << 0x0009;
        const NO_REMOVAL = 1 << 0x000A;
        const CLEANING_MEDIA = 1 << 0x000B;
        const UNSUPPORTED_FORMAT = 1 << 0x000C;
        const RECOVERABLE_MECHANICAL_CARTRIDGE_FAILURE = 1 << 0x000D; // LTO5
        const UNRECOVERABLE_SNAPPED_TAPE = 1 << 0x000E;
        const MEMORY_CHIP_IN_CARTRIDGE_FAILURE = 1 << 0x000F;
        const FORCED_EJECT = 1 << 0x0010;
        const READ_ONLY_FORMAT = 1 << 0x0011;
        const TAPE_DIRECTORY_CORRUPTED = 1 << 0x0012;
        const NEARING_MEDIA_LIFE = 1 << 0x0013;
        const CLEAN_NOW = 1 << 0x0014;
        const CLEAN_PERIODIC = 1 << 0x0015;
        const EXPIRED_CLEANING_MEDIA = 1 << 0x0016;
        const INVALID_CLEANING_TAPE = 1 << 0x0017;
        const RETENSION_REQUEST = 1 << 0x0018; // LTO5
        const HOST_CHANNEL_FAILURE = 1 << 0x0019;
        const COOLING_FAN_FAILURE = 1 << 0x001A;
        const POWER_SUPPLY_FAILURE = 1 << 0x001B;
        const POWER_CONSUMPTION = 1 << 0x001C; // LTO5
        const DRIVE_MANTAINANCE = 1 << 0x001D; // LTO5
        const HARDWARE_A = 1 << 0x001E;
        const HARDWARE_B = 1 << 0x001F;
        const INTERFACE = 1 << 0x0020;
        const EJECT_MEDIA = 1 << 0x0021;
        const DOWNLOAD_FAULT = 1 << 0x0022;
        const DRIVE_HUMIDITY = 1 << 0x0023; // LTO5
        const DRIVE_TEMPERATURE = 1 << 0x0024;
        const DRIVE_VOLTAGE = 1 << 0x0025;
        const PREDICTIVE_FAILURE = 1 << 0x0026;
        const DIAGNOSTICS_REQUIRED = 1 << 0x0027;
        const LOADER_STRAY_TAPE = 1 << 0x0029;
        const LOADER_HARDWARE = 1 << 0x002A;
        const LOADER_MAGAZINE = 1 << 0x002D;
        const DIMINISHED_NATIVE_CAPACITY = 1 << 0x0031;
        const LOST_STATISTICS = 1 << 0x0032;
        const TAPE_DIRECTORY_INVALID_AT_UNLOAD = 1 << 0x0033;
        const TAPE_SYSTEM_AREA_WRITE_FAILURE = 1 << 0x0034;
        const TAPE_SYSTEM_AREA_READ_FAILURE = 1 << 0x0035;
        const NO_START_OF_DATA = 1 << 0x0036;
        const LOADING_FAILURE = 1 << 0x0037;
        const UNRECOVERABLE_UNLOAD_FAILURE = 1 << 0x0038;
        const AUTOMATION_INTERFACE_FAILURE = 1 << 0x0039;
        const FIRMWARE_FAILURE = 1 << 0x003A;
        const WORM_INTEGRITY_CHECK_FAILED = 1 << 0x003B;
        const WORM_OVERWRITE_ATTEMPTED = 1 << 0x003C;
        const ENCRYPTION_POLICY_VIOLATION = 1 << 0x003D;
    }
}

/// Read Tape Alert Flags using raw SCSI command.
pub fn read_tape_alert_flags<F: AsRawFd>(file: &mut F) ->  Result<TapeAlertFlags, Error> {

    let data = sg_read_tape_alert_flags(file)?;

    decode_tape_alert_flags(&data)
}


fn sg_read_tape_alert_flags<F: AsRawFd>(file: &mut F) -> Result<Vec<u8>, Error> {

    let mut sg_raw = SgRaw::new(file, 512)?;

    // Note: We cannjot use LP 2Eh TapeAlerts, because that clears flags on read.
    // Instead, we use LP 12h TapeAlert Response. which does not clear the flags.

    let mut cmd = Vec::new();
    cmd.push(0x4D); // LOG SENSE
    cmd.push(0);
    cmd.push((1<<6) | 0x12); // Tape Alert Response log page
    cmd.push(0);
    cmd.push(0);
    cmd.push(0);
    cmd.push(0);
    cmd.extend(&[2u8, 0u8]); // alloc len
    cmd.push(0u8); // control byte

    sg_raw.do_command(&cmd)
        .map_err(|err| format_err!("read tape alert flags failed - {}", err))
        .map(|v| v.to_vec())
}

fn decode_tape_alert_flags(data: &[u8]) -> Result<TapeAlertFlags, Error> {

    proxmox::try_block!({
        if !((data[0] & 0x7f) == 0x12 && data[1] == 0) {
            bail!("invalid response");
        }

        let mut reader = &data[2..];

        let page_len: u16 = unsafe { reader.read_be_value()? };
        if page_len != 0x0c {
            bail!("invalid page length");
        }

        let parameter_code: u16 = unsafe { reader.read_be_value()? };
        if parameter_code != 0 {
            bail!("invalid parameter code");
        }

        let mut control_buf = [0u8; 2];
        reader.read_exact(&mut control_buf)?;

        if control_buf[1] != 8 {
            bail!("invalid parameter length");
        }

        let value: u64 =  unsafe { reader.read_le_value()? };

        Ok(TapeAlertFlags::from_bits_truncate(value))
    }).map_err(|err| format_err!("decode tape alert flags failed - {}", err))
}
