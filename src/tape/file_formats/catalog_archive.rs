use std::fs::File;
use std::io::Read;

use proxmox_sys::error::SysError;
use proxmox_uuid::Uuid;

use pbs_tape::{MediaContentHeader, TapeWrite, PROXMOX_TAPE_BLOCK_SIZE};

use crate::tape::file_formats::CatalogArchiveHeader;

/// Write a media catalog to the tape
///
/// Returns `Ok(Some(content_uuid))` on success, and `Ok(None)` if
/// `LEOM` was detected before all data was written. The stream is
/// marked inclomplete in that case and does not contain all data (The
/// backup task must rewrite the whole file on the next media).
///
pub fn tape_write_catalog<'a>(
    writer: &mut (dyn TapeWrite + 'a),
    uuid: &Uuid,
    media_set_uuid: &Uuid,
    seq_nr: usize,
    file: &mut File,
    version: [u8; 8],
) -> Result<Option<Uuid>, std::io::Error> {
    let archive_header = CatalogArchiveHeader {
        uuid: uuid.clone(),
        media_set_uuid: media_set_uuid.clone(),
        seq_nr: seq_nr as u64,
    };

    let header_data = serde_json::to_string_pretty(&archive_header)?
        .as_bytes()
        .to_vec();

    let header = MediaContentHeader::new(version, header_data.len() as u32);
    let content_uuid: Uuid = header.uuid.into();

    let leom = writer.write_header(&header, &header_data)?;
    if leom {
        writer.finish(true)?; // mark as incomplete
        return Ok(None);
    }

    let mut file_copy_buffer = proxmox_io::vec::undefined(PROXMOX_TAPE_BLOCK_SIZE);

    let result: Result<(), std::io::Error> = proxmox_lang::try_block!({
        let file_size = file.metadata()?.len();
        let mut remaining = file_size;

        while remaining != 0 {
            let got = file.read(&mut file_copy_buffer[..])?;
            if got as u64 > remaining {
                proxmox_lang::io_bail!("catalog '{}' changed while reading", uuid);
            }
            writer.write_all(&file_copy_buffer[..got])?;
            remaining -= got as u64;
        }
        if remaining > 0 {
            proxmox_lang::io_bail!("catalog '{}' shrunk while reading", uuid);
        }
        Ok(())
    });

    match result {
        Ok(()) => {
            writer.finish(false)?;
            Ok(Some(content_uuid))
        }
        Err(err) => {
            if err.is_errno(nix::errno::Errno::ENOSPC) && writer.logical_end_of_media() {
                writer.finish(true)?; // mark as incomplete
                Ok(None)
            } else {
                Err(err)
            }
        }
    }
}
