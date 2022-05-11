use std::io::{Read, Write};
use std::pin::Pin;
use std::task::{Context, Poll};

use proxmox_sys::error::SysError;
use proxmox_uuid::Uuid;

use pbs_datastore::SnapshotReader;
use pbs_tape::{MediaContentHeader, TapeWrite, PROXMOX_TAPE_BLOCK_SIZE};

use crate::tape::file_formats::{
    SnapshotArchiveHeader, PROXMOX_BACKUP_SNAPSHOT_ARCHIVE_MAGIC_1_1,
    PROXMOX_BACKUP_SNAPSHOT_ARCHIVE_MAGIC_1_2,
};

/// Write a set of files as `pxar` archive to the tape
///
/// This ignores file attributes like ACLs and xattrs.
///
/// Returns `Ok(Some(content_uuid))` on success, and `Ok(None)` if
/// `LEOM` was detected before all data was written. The stream is
/// marked inclomplete in that case and does not contain all data (The
/// backup task must rewrite the whole file on the next media).
pub fn tape_write_snapshot_archive<'a>(
    writer: &mut (dyn TapeWrite + 'a),
    snapshot_reader: &SnapshotReader,
) -> Result<Option<Uuid>, std::io::Error> {
    let backup_dir = snapshot_reader.snapshot();
    let snapshot =
        pbs_api_types::print_ns_and_snapshot(backup_dir.backup_ns(), backup_dir.as_ref());
    let store = snapshot_reader.datastore_name().to_string();
    let file_list = snapshot_reader.file_list();

    let archive_header = SnapshotArchiveHeader { snapshot, store };

    let header_data = serde_json::to_string_pretty(&archive_header)?
        .as_bytes()
        .to_vec();

    let version_magic = if backup_dir.backup_ns().is_root() {
        PROXMOX_BACKUP_SNAPSHOT_ARCHIVE_MAGIC_1_1
    } else {
        PROXMOX_BACKUP_SNAPSHOT_ARCHIVE_MAGIC_1_2
    };

    let header = MediaContentHeader::new(version_magic, header_data.len() as u32);
    let content_uuid = header.uuid.into();

    let root_metadata = pxar::Metadata::dir_builder(0o0664).build();

    let mut file_copy_buffer = proxmox_io::vec::undefined(PROXMOX_TAPE_BLOCK_SIZE);

    let result: Result<(), std::io::Error> = proxmox_lang::try_block!({
        let leom = writer.write_header(&header, &header_data)?;
        if leom {
            return Err(std::io::Error::from_raw_os_error(
                nix::errno::Errno::ENOSPC as i32,
            ));
        }

        let mut encoder =
            pxar::encoder::sync::Encoder::new(PxarTapeWriter::new(writer), &root_metadata)?;

        for filename in file_list.iter() {
            let mut file = snapshot_reader.open_file(filename).map_err(|err| {
                proxmox_lang::io_format_err!("open file '{}' failed - {}", filename, err)
            })?;
            let metadata = file.metadata()?;
            let file_size = metadata.len();

            let metadata: pxar::Metadata = metadata.into();

            if !metadata.is_regular_file() {
                proxmox_lang::io_bail!("file '{}' is not a regular file", filename);
            }

            let mut remaining = file_size;
            let mut out = encoder.create_file(&metadata, filename, file_size)?;
            while remaining != 0 {
                let got = file.read(&mut file_copy_buffer[..])?;
                if got as u64 > remaining {
                    proxmox_lang::io_bail!("file '{}' changed while reading", filename);
                }
                out.write_all(&file_copy_buffer[..got])?;
                remaining -= got as u64;
            }
            if remaining > 0 {
                proxmox_lang::io_bail!("file '{}' shrunk while reading", filename);
            }
        }
        encoder.finish()?;
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

// Helper to create pxar archives on tape
//
// We generate and error at LEOM,
struct PxarTapeWriter<'a, T: TapeWrite + ?Sized> {
    inner: &'a mut T,
}

impl<'a, T: TapeWrite + ?Sized> PxarTapeWriter<'a, T> {
    pub fn new(inner: &'a mut T) -> Self {
        Self { inner }
    }
}

impl<'a, T: TapeWrite + ?Sized> pxar::encoder::SeqWrite for PxarTapeWriter<'a, T> {
    fn poll_seq_write(
        self: Pin<&mut Self>,
        _cx: &mut Context,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let this = unsafe { self.get_unchecked_mut() };
        Poll::Ready(match this.inner.write_all(buf) {
            Ok(leom) => {
                if leom {
                    Err(std::io::Error::from_raw_os_error(
                        nix::errno::Errno::ENOSPC as i32,
                    ))
                } else {
                    Ok(buf.len())
                }
            }
            Err(err) => Err(err),
        })
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}
