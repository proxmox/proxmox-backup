use std::io::Read;
use std::os::unix::io::AsRawFd;

use anyhow::{bail, format_err, Error};

use proxmox_io::ReadExt;

use crate::sgutils2::SgRaw;

bitflags::bitflags! {

    /// Tape Alert Flags
    ///
    /// See LTO SCSI Reference LOG_SENSE - LP 2Eh: TapeAlerts
    pub struct TapeAlertFlags: u64 {
        #[allow(clippy::eq_op)]
        const READ_WARNING = 1 << (0x0001 -1);
        const WRITE_WARNING = 1 << (0x0002 -1);
        const HARD_ERROR = 1 << (0x0003 -1);
        const MEDIA = 1 << (0x0004 -1);
        const READ_FAILURE = 1 << (0x0005 -1);
        const WRITE_FAILURE = 1 << (0x0006 -1);
        const MEDIA_LIFE = 1 << (0x0007 -1);
        const NOT_DATA_GRADE = 1 << (0x0008 -1);
        const WRITE_PROTECT = 1 << (0x0009 -1);
        const NO_REMOVAL = 1 << (0x000A -1);
        const CLEANING_MEDIA = 1 << (0x000B -1);
        const UNSUPPORTED_FORMAT = 1 << (0x000C -1);
        const RECOVERABLE_MECHANICAL_CARTRIDGE_FAILURE = 1 << (0x000D -1); // LTO5
        const UNRECOVERABLE_SNAPPED_TAPE = 1 << (0x000E -1);
        const MEMORY_CHIP_IN_CARTRIDGE_FAILURE = 1 << (0x000F -1);
        const FORCED_EJECT = 1 << (0x0010 -1);
        const READ_ONLY_FORMAT = 1 << (0x0011 -1);
        const TAPE_DIRECTORY_CORRUPTED = 1 << (0x0012 -1);
        const NEARING_MEDIA_LIFE = 1 << (0x0013 -1);
        const CLEAN_NOW = 1 << (0x0014 -1);
        const CLEAN_PERIODIC = 1 << (0x0015 -1);
        const EXPIRED_CLEANING_MEDIA = 1 << (0x0016 -1);
        const INVALID_CLEANING_TAPE = 1 << (0x0017 -1);
        const RETENSION_REQUEST = 1 << (0x0018 -1); // LTO5
        const HOST_CHANNEL_FAILURE = 1 << (0x0019 -1);
        const COOLING_FAN_FAILURE = 1 << (0x001A -1);
        const POWER_SUPPLY_FAILURE = 1 << (0x001B -1);
        const POWER_CONSUMPTION = 1 << (0x001C -1); // LTO5
        const DRIVE_MANTAINANCE = 1 << (0x001D -1); // LTO5
        const HARDWARE_A = 1 << (0x001E -1);
        const HARDWARE_B = 1 << (0x001F -1);
        const INTERFACE = 1 << (0x0020 -1);
        const EJECT_MEDIA = 1 << (0x0021 -1);
        const DOWNLOAD_FAULT = 1 << (0x0022 -1);
        const DRIVE_HUMIDITY = 1 << (0x0023 -1); // LTO5
        const DRIVE_TEMPERATURE = 1 << (0x0024 -1);
        const DRIVE_VOLTAGE = 1 << (0x0025 -1);
        const PREDICTIVE_FAILURE = 1 << (0x0026 -1);
        const DIAGNOSTICS_REQUIRED = 1 << (0x0027 -1);
        const LOADER_STRAY_TAPE = 1 << (0x0029 -1);
        const LOADER_HARDWARE = 1 << (0x002A -1);
        const LOADER_MAGAZINE = 1 << (0x002D -1);
        const DIMINISHED_NATIVE_CAPACITY = 1 << (0x0031 -1);
        const LOST_STATISTICS = 1 << (0x0032 -1);
        const TAPE_DIRECTORY_INVALID_AT_UNLOAD = 1 << (0x0033 -1);
        const TAPE_SYSTEM_AREA_WRITE_FAILURE = 1 << (0x0034 -1);
        const TAPE_SYSTEM_AREA_READ_FAILURE = 1 << (0x0035 -1);
        const NO_START_OF_DATA = 1 << (0x0036 -1);
        const LOADING_FAILURE = 1 << (0x0037 -1);
        const UNRECOVERABLE_UNLOAD_FAILURE = 1 << (0x0038 -1);
        const AUTOMATION_INTERFACE_FAILURE = 1 << (0x0039 -1);
        const FIRMWARE_FAILURE = 1 << (0x003A -1);
        const WORM_INTEGRITY_CHECK_FAILED = 1 << (0x003B -1);
        const WORM_OVERWRITE_ATTEMPTED = 1 << (0x003C -1);
        const ENCRYPTION_POLICY_VIOLATION = 1 << (0x003D -1);
    }
}

/// Read Tape Alert Flags using raw SCSI command.
pub fn read_tape_alert_flags<F: AsRawFd>(file: &mut F) -> Result<TapeAlertFlags, Error> {
    let data = sg_read_tape_alert_flags(file)?;

    decode_tape_alert_flags(&data)
}

#[allow(clippy::vec_init_then_push)]
fn sg_read_tape_alert_flags<F: AsRawFd>(file: &mut F) -> Result<Vec<u8>, Error> {
    let mut sg_raw = SgRaw::new(file, 512)?;

    // Note: We cannjot use LP 2Eh TapeAlerts, because that clears flags on read.
    // Instead, we use LP 12h TapeAlert Response. which does not clear the flags.

    let mut cmd = Vec::new();
    cmd.push(0x4D); // LOG SENSE
    cmd.push(0);
    cmd.push((1 << 6) | 0x12); // Tape Alert Response log page
    cmd.push(0);
    cmd.push(0);
    cmd.push(0);
    cmd.push(0);
    cmd.extend([2u8, 0u8]); // alloc len
    cmd.push(0u8); // control byte

    sg_raw
        .do_command(&cmd)
        .map_err(|err| format_err!("read tape alert flags failed - {}", err))
        .map(|v| v.to_vec())
}

fn decode_tape_alert_flags(data: &[u8]) -> Result<TapeAlertFlags, Error> {
    proxmox_lang::try_block!({
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

        let mut value: u64 = unsafe { reader.read_be_value()? };

        // bits are in wrong order, reverse them
        value = value.reverse_bits();

        Ok(TapeAlertFlags::from_bits_truncate(value))
    })
    .map_err(|err| format_err!("decode tape alert flags failed - {}", err))
}

const CRITICAL_FLAG_MASK: u64 = TapeAlertFlags::MEDIA.bits()
    | TapeAlertFlags::WRITE_FAILURE.bits()
    | TapeAlertFlags::READ_FAILURE.bits()
    | TapeAlertFlags::WRITE_PROTECT.bits()
    | TapeAlertFlags::UNRECOVERABLE_SNAPPED_TAPE.bits()
    | TapeAlertFlags::FORCED_EJECT.bits()
    | TapeAlertFlags::EXPIRED_CLEANING_MEDIA.bits()
    | TapeAlertFlags::INVALID_CLEANING_TAPE.bits()
    | TapeAlertFlags::HARDWARE_A.bits()
    | TapeAlertFlags::HARDWARE_B.bits()
    | TapeAlertFlags::EJECT_MEDIA.bits()
    | TapeAlertFlags::PREDICTIVE_FAILURE.bits()
    | TapeAlertFlags::LOADER_STRAY_TAPE.bits()
    | TapeAlertFlags::LOADER_MAGAZINE.bits()
    | TapeAlertFlags::TAPE_SYSTEM_AREA_WRITE_FAILURE.bits()
    | TapeAlertFlags::TAPE_SYSTEM_AREA_READ_FAILURE.bits()
    | TapeAlertFlags::NO_START_OF_DATA.bits()
    | TapeAlertFlags::LOADING_FAILURE.bits()
    | TapeAlertFlags::UNRECOVERABLE_UNLOAD_FAILURE.bits()
    | TapeAlertFlags::AUTOMATION_INTERFACE_FAILURE.bits();

/// Check if tape-alert-flags contains critial errors.
pub fn tape_alert_flags_critical(flags: TapeAlertFlags) -> bool {
    (flags.bits() & CRITICAL_FLAG_MASK) != 0
}

const MEDIA_LIFE_MASK: u64 =
    TapeAlertFlags::MEDIA_LIFE.bits() | TapeAlertFlags::NEARING_MEDIA_LIFE.bits();

/// Check if tape-alert-flags indicates media-life end
pub fn tape_alert_flags_media_life(flags: TapeAlertFlags) -> bool {
    (flags.bits() & MEDIA_LIFE_MASK) != 0
}

const MEDIA_CLEAN_MASK: u64 =
    TapeAlertFlags::CLEAN_NOW.bits() | TapeAlertFlags::CLEAN_PERIODIC.bits();

/// Check if tape-alert-flags indicates media cleaning request
pub fn tape_alert_flags_cleaning_request(flags: TapeAlertFlags) -> bool {
    (flags.bits() & MEDIA_CLEAN_MASK) != 0
}
