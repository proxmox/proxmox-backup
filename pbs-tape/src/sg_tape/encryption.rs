use std::io::Write;
use std::os::unix::prelude::AsRawFd;

use anyhow::{bail, format_err, Error};
use endian_trait::Endian;

use proxmox_io::{ReadExt, WriteExt};

use crate::sgutils2::{alloc_page_aligned_buffer, SgRaw};

/// Test if drive supports hardware encryption
///
/// We search for AES_GCM algorithm with 256bits key.
pub fn has_encryption<F: AsRawFd>(file: &mut F) -> bool {
    let data = match sg_spin_data_encryption_caps(file) {
        Ok(data) => data,
        Err(_) => return false,
    };
    decode_spin_data_encryption_caps(&data).is_ok()
}

/// Set or clear encryption key
///
/// We always use mixed mode,
pub fn set_encryption<F: AsRawFd>(file: &mut F, key: Option<[u8; 32]>) -> Result<(), Error> {
    let data = match sg_spin_data_encryption_caps(file) {
        Ok(data) => data,
        Err(_) if key.is_none() => {
            // Assume device does not support HW encryption
            // We can simply ignore the clear key request
            return Ok(());
        }
        Err(err) => return Err(err),
    };

    let algorithm_index = decode_spin_data_encryption_caps(&data)?;

    sg_spout_set_encryption(file, algorithm_index, key)?;

    let data = sg_spin_data_encryption_status(file)?;
    let status = decode_spin_data_encryption_status(&data)?;

    match status.mode {
        DataEncryptionMode::Off => {
            if key.is_none() {
                return Ok(());
            }
        }
        DataEncryptionMode::Mixed => {
            if key.is_some() {
                return Ok(());
            }
        }
        _ => {}
    }

    bail!("got unexpected encryption mode {:?}", status.mode);
}

#[derive(Endian)]
#[repr(C, packed)]
struct SspSetDataEncryptionPage {
    page_code: u16,
    page_len: u16,
    scope_byte: u8,
    control_byte_5: u8,
    encryption_mode: u8,
    decryption_mode: u8,
    algorythm_index: u8,
    key_format: u8,
    reserved: [u8; 8],
    key_len: u16,
    /* key follows */
}

#[allow(clippy::vec_init_then_push)]
fn sg_spout_set_encryption<F: AsRawFd>(
    file: &mut F,
    algorythm_index: u8,
    key: Option<[u8; 32]>,
) -> Result<(), Error> {
    let mut sg_raw = SgRaw::new(file, 0)?;

    let mut outbuf_len = std::mem::size_of::<SspSetDataEncryptionPage>();
    if let Some(ref key) = key {
        outbuf_len += key.len();
    }

    let mut outbuf = alloc_page_aligned_buffer(outbuf_len)?;
    let chok: u8 = 0;

    let page = SspSetDataEncryptionPage {
        page_code: 0x10,
        page_len: (outbuf_len - 4) as u16,
        scope_byte: (0b10 << 5), // all IT nexus
        control_byte_5: (chok << 2),
        encryption_mode: if key.is_some() { 2 } else { 0 },
        decryption_mode: if key.is_some() { 3 } else { 0 }, // mixed mode
        algorythm_index,
        key_format: 0,
        reserved: [0u8; 8],
        key_len: if let Some(ref key) = key {
            key.len() as u16
        } else {
            0
        },
    };

    let mut writer = &mut outbuf[..];
    unsafe { writer.write_be_value(page)? };

    if let Some(ref key) = key {
        writer.write_all(key)?;
    }

    let mut cmd = Vec::new();
    cmd.push(0xB5); // SECURITY PROTOCOL IN (SPOUT)
    cmd.push(0x20); // Tape Data Encryption Page
    cmd.push(0);
    cmd.push(0x10); // Set Data Encryption page
    cmd.push(0);
    cmd.push(0);
    cmd.extend((outbuf_len as u32).to_be_bytes()); // data out len
    cmd.push(0);
    cmd.push(0);

    sg_raw
        .do_out_command(&cmd, &outbuf)
        .map_err(|err| format_err!("set data encryption SPOUT(20h[0010h]) failed - {}", err))
}

// Warning: this blocks and fails if there is no media loaded
#[allow(clippy::vec_init_then_push)]
fn sg_spin_data_encryption_status<F: AsRawFd>(file: &mut F) -> Result<Vec<u8>, Error> {
    let allocation_len: u32 = 8192 + 4;

    let mut sg_raw = SgRaw::new(file, allocation_len as usize)?;

    let mut cmd = Vec::new();
    cmd.push(0xA2); // SECURITY PROTOCOL IN (SPIN)
    cmd.push(0x20); // Tape Data Encryption Page
    cmd.push(0);
    cmd.push(0x20); // Data Encryption Status page
    cmd.push(0);
    cmd.push(0);
    cmd.extend(allocation_len.to_be_bytes());
    cmd.push(0);
    cmd.push(0);

    sg_raw
        .do_command(&cmd)
        .map_err(|err| {
            format_err!(
                "read data encryption status SPIN(20h[0020h]) failed - {}",
                err
            )
        })
        .map(|v| v.to_vec())
}

// Warning: this blocks and fails if there is no media loaded
#[allow(clippy::vec_init_then_push)]
fn sg_spin_data_encryption_caps<F: AsRawFd>(file: &mut F) -> Result<Vec<u8>, Error> {
    let allocation_len: u32 = 8192 + 4;

    let mut sg_raw = SgRaw::new(file, allocation_len as usize)?;

    let mut cmd = Vec::new();
    cmd.push(0xA2); // SECURITY PROTOCOL IN (SPIN)
    cmd.push(0x20); // Tape Data Encryption Page
    cmd.push(0);
    cmd.push(0x10); // Data Encryption Capabilities page
    cmd.push(0);
    cmd.push(0);
    cmd.extend(allocation_len.to_be_bytes());
    cmd.push(0);
    cmd.push(0);

    sg_raw
        .do_command(&cmd)
        .map_err(|err| {
            format_err!(
                "read data encryption caps SPIN(20h[0010h]) failed - {}",
                err
            )
        })
        .map(|v| v.to_vec())
}

#[derive(Debug)]
enum DataEncryptionMode {
    On,
    Mixed,
    RawRead,
    Off,
}

#[derive(Debug)]
struct DataEncryptionStatus {
    mode: DataEncryptionMode,
}

#[derive(Endian)]
#[repr(C, packed)]
struct SspDataEncryptionCapabilityPage {
    page_code: u16,
    page_len: u16,
    reserved: [u8; 16],
}

#[derive(Endian)]
#[repr(C, packed)]
struct SspDataEncryptionAlgorithmDescriptor {
    algorythm_index: u8,
    reserved1: u8,
    descriptor_len: u16,
    control_byte_4: u8,
    control_byte_5: u8,
    max_ucad_bytes: u16,
    max_acad_bytes: u16,
    key_size: u16,
    control_byte_12: u8,
    reserved2: u8,
    msdk_count: u16,
    reserved3: [u8; 4],
    algorithm_code: u32,
}

// Returns the algorythm_index for AES-GCM
fn decode_spin_data_encryption_caps(data: &[u8]) -> Result<u8, Error> {
    proxmox_lang::try_block!({
        let mut reader = data;
        let _page: SspDataEncryptionCapabilityPage = unsafe { reader.read_be_value()? };

        let mut aes_gcm_index = None;

        loop {
            if reader.is_empty() {
                break;
            };
            let desc: SspDataEncryptionAlgorithmDescriptor = unsafe { reader.read_be_value()? };
            if desc.descriptor_len != 0x14 {
                bail!("got wrong key descriptor len");
            }
            if (desc.control_byte_4 & 0b00000011) != 2 {
                continue; // can't encrypt in hardware
            }
            if ((desc.control_byte_4 & 0b00001100) >> 2) != 2 {
                continue; // can't decrypt in hardware
            }
            if desc.algorithm_code == 0x00010014 && desc.key_size == 32 {
                aes_gcm_index = Some(desc.algorythm_index);
                break;
            }
        }

        match aes_gcm_index {
            Some(index) => Ok(index),
            None => bail!("drive does not support AES-GCM encryption"),
        }
    })
    .map_err(|err: Error| format_err!("decode data encryption caps page failed - {}", err))
}

#[derive(Endian)]
#[repr(C, packed)]
struct SspDataEncryptionStatusPage {
    page_code: u16,
    page_len: u16,
    scope_byte: u8,
    encryption_mode: u8,
    decryption_mode: u8,
    algorythm_index: u8,
    key_instance_counter: u32,
    control_byte: u8,
    key_format: u8,
    key_len: u16,
    reserved: [u8; 8],
}

fn decode_spin_data_encryption_status(data: &[u8]) -> Result<DataEncryptionStatus, Error> {
    proxmox_lang::try_block!({
        let mut reader = data;
        let page: SspDataEncryptionStatusPage = unsafe { reader.read_be_value()? };

        if page.page_code != 0x20 {
            bail!("invalid response");
        }

        let mode = match (page.encryption_mode, page.decryption_mode) {
            (0, 0) => DataEncryptionMode::Off,
            (2, 1) => DataEncryptionMode::RawRead,
            (2, 2) => DataEncryptionMode::On,
            (2, 3) => DataEncryptionMode::Mixed,
            _ => bail!("unknown encryption mode"),
        };

        let status = DataEncryptionStatus { mode };

        Ok(status)
    })
    .map_err(|err| format_err!("decode data encryption status page failed - {}", err))
}
