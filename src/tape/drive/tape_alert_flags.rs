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
        const READ_WARNING = 0x0001;
        const WRITE_WARNING = 0002;
        const HARD_ERROR = 0x0003;
        const MEDIA = 0x0004;
        const READ_FAILURE = 0x0005;
        const WRITE_FAILURE = 0x0006;
        const MEDIA_LIFE = 0x0007;
        const NOT_DATA_GRADE = 0x0008;
        const WRITE_PROTECT = 0x0009;
        const NO_REMOVAL = 0x000A;
        const CLEANING_MEDIA = 0x000B;
        const UNSUPPORTED_FORMAT = 0x000C;
        const UNRECOVERABLE_SNAPPED_TAPE = 0x000E;
        const MEMORY_CHIP_IN_CARTRIDGE_FAILURE = 0x000F;
        const FORCED_EJECT = 0x0010;
        const READ_ONLY_FORMAT = 0x0011;
        const TAPE_DIRECTORY_CORRUPTED = 0x0012;
        const NEARING_MEDIA_LIFE = 0x0013;
        const CLEAN_NOW = 0x0014;
        const CLEAN_PERIODIC = 0x0015;
        const EXPIRED_CLEANING_MEDIA = 0x0016;
        const INVALID_CLEANING_TAPE = 0x0017;
        const HOST_CHANNEL_FAILURE = 0x0019;
        const COOLING_FAN_FAILURE = 0x001A;
        const POWER_SUPPLY_FAILURE = 0x001B;
        const HARDWARE_A = 0x001E;
        const HARDWARE_B = 0x001F;
        const INTERFACE = 0x0020;
        const EJECT_MEDIA = 0x0021;
        const DOWNLOAD_FAULT = 0x0022;
        const DRIVE_TEMPERATURE = 0x0024;
        const DRIVE_VOLTAGE = 0x0025;
        const PREDICTIVE_FAILURE = 0x0026;
        const DIAGNOSTICS_REQUIRED = 0x0027;
        const LOADER_STRAY_TAPE = 0x0029;
        const LOADER_HARDWARE = 0x002A;
        const LOADER_MAGAZINE = 0x002D;
        const DIMINISHED_NATIVE_CAPACITY = 0x0031;
        const LOST_STATISTICS = 0x0032;
        const TAPE_DIRECTORY_INVALID_AT_UNLOAD = 0x0033;
        const TAPE_SYSTEM_AREA_WRITE_FAILURE = 0x0034;
        const TAPE_SYSTEM_AREA_READ_FAILURE = 0x0035;
        const NO_START_OF_DATA = 0x0036;
        const LOADING_FAILURE = 0x0037;
        const UNRECOVERABLE_UNLOAD_FAILURE = 0x0038;
        const AUTOMATION_INTERFACE_FAILURE = 0x0039;
        const FIRMWARE_FAILURE = 0x003A;
        const WORM_INTEGRITY_CHECK_FAILED = 0x003B;
        const WORM_OVERWRITE_ATTEMPTED = 0x003C;
        const ENCRYPTION_POLICY_VIOLATION = 0x003D;
    }
}

/// Read Tape Alert Flags using raw SCSI command.
pub fn read_tape_alert_flags<F: AsRawFd>(file: &mut F) ->  Result<TapeAlertFlags, Error> {

    let data = sg_read_tape_alert_flags(file)?;

    decode_tape_alert_flags(&data)
}

fn sg_read_tape_alert_flags<F: AsRawFd>(file: &mut F) -> Result<Vec<u8>, Error> {

    let mut sg_raw = SgRaw::new(file, 512)?;

    let mut cmd = Vec::new();
    cmd.push(0x4D); // LOG SENSE
    cmd.push(0);
    cmd.push(0x2e); // Tape Alert Flag page
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
        if !(data[0] == 0x2e && data[1] == 0) {
            bail!("invalid response");
        }

        let mut reader = &data[2..];

        let page_len: u16 = unsafe { reader.read_be_value()? };
        if page_len != 0x140 {
            bail!("invalid page length");
        }

        let mut value: u64 = 0;

        for _ in 1..65 {
            let id: u16 = unsafe { reader.read_be_value()? };
            if id < 1 || id > 64 {
                bail!("invalid parameter id '{}'", id);
            }
            let bit: u64 = 1 << (id as usize - 1);
            let mut data = [0u8;3];
            reader.read_exact(&mut data)?;
            if data[1] != 1 {
                bail!("invalid parameter length");
            }
            match data[2] {
                0 => {},
                1 => { value |= bit; }
                _ => bail!("invalid flag value"),
            }
        }

        Ok(TapeAlertFlags::from_bits_truncate(value))
    }).map_err(|err| format_err!("decode tape alert flags failed - {}", err))
}
