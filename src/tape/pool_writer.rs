use std::collections::HashSet;
use std::path::Path;

use anyhow::{bail, Error};

use proxmox::tools::Uuid;

use crate::{
    backup::{
        DataStore,
    },
    server::WorkerTask,
    tape::{
        TAPE_STATUS_DIR,
        MAX_CHUNK_ARCHIVE_SIZE,
        COMMIT_BLOCK_SIZE,
        TapeDriver,
        TapeWrite,
        ChunkArchiveWriter,
        SnapshotReader,
        SnapshotChunkIterator,
        MediaPool,
        MediaId,
        MediaCatalog,
        MediaSetCatalog,
        tape_write_snapshot_archive,
        request_and_load_media,
        tape_alert_flags_critical,
        media_changer,
        file_formats::MediaSetLabel,
    },
};


struct PoolWriterState {
    drive: Box<dyn TapeDriver>,
    catalog: MediaCatalog,
    // tell if we already moved to EOM
    at_eom: bool,
    // bytes written after the last tape fush/sync
    bytes_written: usize,
}

impl PoolWriterState {

    fn commit(&mut self) -> Result<(), Error> {
        self.drive.sync()?; // sync all data to the tape
        self.catalog.commit()?; // then commit the catalog
        self.bytes_written = 0;
        Ok(())
    }
}

/// Helper to manage a backup job, writing several tapes of a pool
pub struct PoolWriter {
    pool: MediaPool,
    drive_name: String,
    status: Option<PoolWriterState>,
    media_set_catalog: MediaSetCatalog,
}

impl PoolWriter {

    pub fn new(mut pool: MediaPool, drive_name: &str) -> Result<Self, Error> {

        let current_time = proxmox::tools::time::epoch_i64();

        pool.start_write_session(current_time)?;

        let mut media_set_catalog = MediaSetCatalog::new();

        // load all catalogs read-only at start
        for media_uuid in pool.current_media_list()? {
            let media_catalog = MediaCatalog::open(
                Path::new(TAPE_STATUS_DIR),
                &media_uuid,
                false,
                false,
            )?;
            media_set_catalog.append_catalog(media_catalog)?;
        }

        Ok(Self {
            pool,
            drive_name: drive_name.to_string(),
            status: None,
            media_set_catalog,
         })
    }

    pub fn pool(&mut self) -> &mut MediaPool {
        &mut self.pool
    }

    /// Set media status to FULL (persistent - stores pool status)
    pub fn set_media_status_full(&mut self, uuid: &Uuid) -> Result<(), Error> {
        self.pool.set_media_status_full(&uuid)?;
        Ok(())
    }

    pub fn contains_snapshot(&self, snapshot: &str) -> bool {
        if let Some(PoolWriterState { ref catalog, .. }) = self.status {
            if catalog.contains_snapshot(snapshot) {
                return true;
            }
        }
        self.media_set_catalog.contains_snapshot(snapshot)
    }

    /// Eject media and drop PoolWriterState (close drive)
    pub fn eject_media(&mut self) -> Result<(), Error> {
        let mut status = match self.status.take() {
            Some(status) => status,
            None => return Ok(()), // no media loaded
        };

        let (drive_config, _digest) = crate::config::drive::config()?;

        if let Some((mut changer, _)) = media_changer(&drive_config, &self.drive_name)? {
            changer.unload_media(None)?;
        } else {
            status.drive.eject_media()?;
        }

        Ok(())
    }

    /// commit changes to tape and catalog
    ///
    /// This is done automatically during a backupsession, but needs to
    /// be called explicitly before dropping the PoolWriter
    pub fn commit(&mut self) -> Result<(), Error> {
        if let Some(ref mut status) = self.status {
            status.commit()?;
        }
        Ok(())
    }

    /// Load a writable media into the drive
    pub fn load_writable_media(&mut self, worker: &WorkerTask) -> Result<Uuid, Error> {
        let last_media_uuid = match self.status {
            Some(PoolWriterState { ref catalog, .. }) => Some(catalog.uuid().clone()),
            None => None,
        };

        let current_time = proxmox::tools::time::epoch_i64();
        let media_uuid = self.pool.alloc_writable_media(current_time)?;

        let media = self.pool.lookup_media(&media_uuid).unwrap();

        let media_changed = match last_media_uuid {
            Some(ref last_media_uuid) => last_media_uuid != &media_uuid,
            None => true,
        };

        if !media_changed {
            return Ok(media_uuid);
        }

        // remove read-only catalog (we store a writable version in status)
        self.media_set_catalog.remove_catalog(&media_uuid);

        if let Some(PoolWriterState {mut drive, catalog, .. }) = self.status.take() {
            self.media_set_catalog.append_catalog(catalog)?;
            drive.eject_media()?;
        }

        let (drive_config, _digest) = crate::config::drive::config()?;

        let (mut drive, old_media_id) =
            request_and_load_media(worker, &drive_config, &self.drive_name, media.label())?;

        // test for critical tape alert flags
        let alert_flags = drive.tape_alert_flags()?;
        if !alert_flags.is_empty() {
            worker.log(format!("TapeAlertFlags: {:?}", alert_flags));
            if tape_alert_flags_critical(alert_flags) {
                bail!("aborting due to critical tape alert flags: {:?}", alert_flags);
            }
        }

        let catalog = update_media_set_label(
            worker,
            drive.as_mut(),
            old_media_id.media_set_label,
            media.id(),
        )?;

        self.status = Some(PoolWriterState { drive, catalog, at_eom: false, bytes_written: 0 });

        Ok(media_uuid)
    }

    /// uuid of currently loaded BackupMedia
    pub fn current_media_uuid(&self) -> Result<&Uuid, Error> {
        match self.status {
            Some(PoolWriterState { ref catalog, ..}) => Ok(catalog.uuid()),
            None => bail!("PoolWriter - no media loaded"),
        }
    }

    /// Move to EOM (if not aleady there), then creates a new snapshot
    /// archive writing specified files (as .pxar) into it. On
    /// success, this return 'Ok(true)' and the media catalog gets
    /// updated.

    /// Please note that this may fail when there is not enough space
    /// on the media (return value 'Ok(false, _)'). In that case, the
    /// archive is marked incomplete, and we do not use it. The caller
    /// should mark the media as full and try again using another
    /// media.
    pub fn append_snapshot_archive(
        &mut self,
        snapshot_reader: &SnapshotReader,
    ) -> Result<(bool, usize), Error> {

        let status = match self.status {
            Some(ref mut status) => status,
            None => bail!("PoolWriter - no media loaded"),
        };

        if !status.at_eom {
            status.drive.move_to_eom()?;
            status.at_eom = true;
        }

        let current_file_number = status.drive.current_file_number()?;
        if current_file_number < 2 {
            bail!("got strange file position number from drive ({})", current_file_number);
        }

        let (done, bytes_written) = {
            let mut writer: Box<dyn TapeWrite> = status.drive.write_file()?;

            match tape_write_snapshot_archive(writer.as_mut(), snapshot_reader)? {
                Some(content_uuid) => {
                    status.catalog.register_snapshot(
                        content_uuid,
                        current_file_number,
                        &snapshot_reader.snapshot().to_string(),
                    )?;
                    (true, writer.bytes_written())
                }
                None => (false, writer.bytes_written()),
            }
        };

        status.bytes_written += bytes_written;

        let request_sync = if status.bytes_written >= COMMIT_BLOCK_SIZE { true } else { false };

        if !done || request_sync {
            status.commit()?;
        }

        Ok((done, bytes_written))
    }

    /// Move to EOM (if not aleady there), then creates a new chunk
    /// archive and writes chunks from 'chunk_iter'. This stops when
    /// it detect LEOM or when we reach max archive size
    /// (4GB). Written chunks are registered in the media catalog.
    pub fn append_chunk_archive(
        &mut self,
        datastore: &DataStore,
        chunk_iter: &mut std::iter::Peekable<SnapshotChunkIterator>,
    ) -> Result<(bool, usize), Error> {

        let status = match self.status {
            Some(ref mut status) => status,
            None => bail!("PoolWriter - no media loaded"),
        };

        if !status.at_eom {
            status.drive.move_to_eom()?;
            status.at_eom = true;
        }

        let current_file_number = status.drive.current_file_number()?;
        if current_file_number < 2 {
            bail!("got strange file position number from drive ({})", current_file_number);
        }
        let writer = status.drive.write_file()?;

        let (saved_chunks, content_uuid, leom, bytes_written) = write_chunk_archive(
            writer,
            datastore,
            chunk_iter,
            &self.media_set_catalog,
            &status.catalog,
            MAX_CHUNK_ARCHIVE_SIZE,
        )?;

        status.bytes_written += bytes_written;

        let request_sync = if status.bytes_written >= COMMIT_BLOCK_SIZE { true } else { false };

        // register chunks in media_catalog
        status.catalog.start_chunk_archive(content_uuid, current_file_number)?;
        for digest in saved_chunks {
            status.catalog.register_chunk(&digest)?;
        }
        status.catalog.end_chunk_archive()?;

        if leom || request_sync {
            status.commit()?;
        }

        Ok((leom, bytes_written))
    }
}

/// write up to <max_size> of chunks
fn write_chunk_archive<'a>(
    writer: Box<dyn 'a + TapeWrite>,
    datastore: &DataStore,
    chunk_iter: &mut std::iter::Peekable<SnapshotChunkIterator>,
    media_set_catalog: &MediaSetCatalog,
    media_catalog: &MediaCatalog,
    max_size: usize,
) -> Result<(Vec<[u8;32]>, Uuid, bool, usize), Error> {

    let (mut writer, content_uuid) = ChunkArchiveWriter::new(writer, true)?;

    let mut chunk_index: HashSet<[u8;32]> = HashSet::new();

    // we want to get the chunk list in correct order
    let mut chunk_list: Vec<[u8;32]> = Vec::new();

    let mut leom = false;

    loop {
        let digest = match chunk_iter.next() {
            None => break,
            Some(digest) => digest?,
        };
        if media_catalog.contains_chunk(&digest)
            || chunk_index.contains(&digest)
            || media_set_catalog.contains_chunk(&digest)
        {
            continue;
        }

        let blob = datastore.load_chunk(&digest)?;
        println!("CHUNK {} size {}", proxmox::tools::digest_to_hex(&digest), blob.raw_size());

        match writer.try_write_chunk(&digest, &blob) {
            Ok(true) => {
                chunk_index.insert(digest);
                chunk_list.push(digest);
            }
            Ok(false) => {
                leom = true;
                break;
            }
            Err(err) => bail!("write chunk failed - {}", err),
        }

        if writer.bytes_written() > max_size {
            println!("Chunk Archive max size reached, closing archive");
            break;
        }
    }

    writer.finish()?;

    Ok((chunk_list, content_uuid, leom, writer.bytes_written()))
}

// Compare the media set label. If the media is empty, or the existing
// set label does not match the expected media set, overwrite the
// media set label.
fn update_media_set_label(
    worker: &WorkerTask,
    drive: &mut dyn TapeDriver,
    old_set: Option<MediaSetLabel>,
    media_id: &MediaId,
) -> Result<MediaCatalog, Error> {

    let media_catalog;

    let new_set = match media_id.media_set_label {
        None => bail!("got media without media set - internal error"),
        Some(ref set) => set,
    };

    let status_path = Path::new(TAPE_STATUS_DIR);

    match old_set {
        None => {
            worker.log(format!("wrinting new media set label"));
            drive.write_media_set_label(new_set)?;
            media_catalog = MediaCatalog::overwrite(status_path, media_id, true)?;
        }
        Some(media_set_label) => {
            if new_set.uuid == media_set_label.uuid {
                if new_set.seq_nr != media_set_label.seq_nr {
                    bail!("got media with wrong media sequence number ({} != {}",
                          new_set.seq_nr,media_set_label.seq_nr);
                }
                media_catalog = MediaCatalog::open(status_path, &media_id.label.uuid, true, false)?;
            } else {
                worker.log(
                    format!("wrinting new media set label (overwrite '{}/{}')",
                            media_set_label.uuid.to_string(), media_set_label.seq_nr)
                );

                drive.write_media_set_label(new_set)?;
                media_catalog = MediaCatalog::overwrite(status_path, media_id, true)?;
            }
        }
    }

    // todo: verify last content/media_catalog somehow?
    drive.move_to_eom()?;

    Ok(media_catalog)
}
