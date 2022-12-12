mod catalog_set;
pub use catalog_set::*;

mod new_chunks_iterator;
pub use new_chunks_iterator::*;

use std::collections::HashSet;
use std::fs::File;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use anyhow::{bail, Error};

use proxmox_sys::{task_log, task_warn};
use proxmox_uuid::Uuid;

use pbs_datastore::{DataStore, SnapshotReader};
use pbs_tape::{sg_tape::tape_alert_flags_critical, TapeWrite};
use proxmox_rest_server::WorkerTask;

use crate::tape::{
    drive::{media_changer, request_and_load_media, TapeDriver},
    encryption_keys::load_key_configs,
    file_formats::{
        tape_write_catalog, tape_write_snapshot_archive, ChunkArchiveWriter, MediaSetLabel,
    },
    MediaCatalog, MediaId, MediaPool, COMMIT_BLOCK_SIZE, MAX_CHUNK_ARCHIVE_SIZE, TAPE_STATUS_DIR,
};

use super::file_formats::{
    PROXMOX_BACKUP_CATALOG_ARCHIVE_MAGIC_1_0, PROXMOX_BACKUP_CATALOG_ARCHIVE_MAGIC_1_1,
};

struct PoolWriterState {
    drive: Box<dyn TapeDriver>,
    // Media Uuid from loaded media
    media_uuid: Uuid,
    // tell if we already moved to EOM
    at_eom: bool,
    // bytes written after the last tape fush/sync
    bytes_written: usize,
}

/// Helper to manage a backup job, writing several tapes of a pool
pub struct PoolWriter {
    pool: MediaPool,
    drive_name: String,
    status: Option<PoolWriterState>,
    catalog_set: Arc<Mutex<CatalogSet>>,
    notify_email: Option<String>,
    ns_magic: bool,
    used_tapes: HashSet<Uuid>,
}

impl PoolWriter {
    pub fn new(
        mut pool: MediaPool,
        drive_name: &str,
        worker: &WorkerTask,
        notify_email: Option<String>,
        force_media_set: bool,
        ns_magic: bool,
    ) -> Result<Self, Error> {
        let current_time = proxmox_time::epoch_i64();

        let new_media_set_reason = pool.start_write_session(current_time, force_media_set)?;
        if let Some(reason) = new_media_set_reason {
            task_log!(worker, "starting new media set - reason: {}", reason,);
        }

        let media_set_uuid = pool.current_media_set().uuid();
        task_log!(worker, "media set uuid: {}", media_set_uuid);

        let mut catalog_set = CatalogSet::new();

        // load all catalogs read-only at start
        for media_uuid in pool.current_media_list()? {
            let media_info = pool.lookup_media(media_uuid).unwrap();
            let media_catalog = MediaCatalog::open(TAPE_STATUS_DIR, media_info.id(), false, false)?;
            catalog_set.append_read_only_catalog(media_catalog)?;
        }

        Ok(Self {
            pool,
            drive_name: drive_name.to_string(),
            status: None,
            catalog_set: Arc::new(Mutex::new(catalog_set)),
            notify_email,
            ns_magic,
            used_tapes: HashSet::new(),
        })
    }

    pub fn pool(&mut self) -> &mut MediaPool {
        &mut self.pool
    }

    /// Set media status to FULL (persistent - stores pool status)
    pub fn set_media_status_full(&mut self, uuid: &Uuid) -> Result<(), Error> {
        self.pool.set_media_status_full(uuid)?;
        Ok(())
    }

    pub fn get_used_media_labels(&self) -> Result<Vec<String>, Error> {
        let mut res = Vec::with_capacity(self.used_tapes.len());
        for media_uuid in &self.used_tapes {
            let media_info = self.pool.lookup_media(media_uuid)?;
            res.push(media_info.label_text().to_string());
        }

        Ok(res)
    }

    pub fn contains_snapshot(
        &self,
        store: &str,
        ns: &pbs_api_types::BackupNamespace,
        snapshot: &pbs_api_types::BackupDir,
    ) -> bool {
        self.catalog_set
            .lock()
            .unwrap()
            .contains_snapshot(store, ns, snapshot)
    }

    /// Eject media and drop PoolWriterState (close drive)
    pub fn eject_media(&mut self, worker: &WorkerTask) -> Result<(), Error> {
        let mut status = match self.status.take() {
            Some(status) => status,
            None => return Ok(()), // no media loaded
        };

        let (drive_config, _digest) = pbs_config::drive::config()?;

        if let Some((mut changer, _)) = media_changer(&drive_config, &self.drive_name)? {
            task_log!(worker, "eject media");
            status.drive.eject_media()?; // rewind and eject early, so that unload_media is faster
            drop(status); // close drive
            task_log!(worker, "unload media");
            changer.unload_media(None)?; //eject and unload
        } else {
            task_log!(worker, "standalone drive - ejecting media");
            status.drive.eject_media()?;
        }

        Ok(())
    }

    /// Export current media set and drop PoolWriterState (close drive)
    pub fn export_media_set(&mut self, worker: &WorkerTask) -> Result<(), Error> {
        let mut status = self.status.take();

        let (drive_config, _digest) = pbs_config::drive::config()?;

        if let Some((mut changer, _)) = media_changer(&drive_config, &self.drive_name)? {
            if let Some(ref mut status) = status {
                task_log!(worker, "rewind media");
                // rewind first so that the unload command later does not run into a timeout
                status.drive.rewind()?;
            }
            drop(status); // close drive

            for media_uuid in self.pool.current_media_list()? {
                let media = self.pool.lookup_media(media_uuid)?;
                let label_text = media.label_text();
                if let Some(slot) = changer.export_media(label_text)? {
                    task_log!(
                        worker,
                        "exported media '{}' to import/export slot {}",
                        label_text,
                        slot
                    );
                } else {
                    task_warn!(
                        worker,
                        "export failed - media '{}' is not online or in different drive",
                        label_text
                    );
                }
            }
        } else if let Some(mut status) = status {
            task_log!(
                worker,
                "standalone drive - ejecting media instead of export"
            );
            status.drive.eject_media()?;
        }

        Ok(())
    }

    /// commit changes to tape and catalog
    ///
    /// This is done automatically during a backupsession, but needs to
    /// be called explicitly before dropping the PoolWriter
    pub fn commit(&mut self) -> Result<(), Error> {
        if let Some(PoolWriterState { ref mut drive, .. }) = self.status {
            drive.sync()?; // sync all data to the tape
        }
        self.catalog_set.lock().unwrap().commit()?; // then commit the catalog
        Ok(())
    }

    /// Load a writable media into the drive
    pub fn load_writable_media(&mut self, worker: &WorkerTask) -> Result<Uuid, Error> {
        let last_media_uuid = match self.status {
            Some(PoolWriterState { ref media_uuid, .. }) => Some(media_uuid.clone()),
            None => None,
        };

        let current_time = proxmox_time::epoch_i64();
        let media_uuid = self.pool.alloc_writable_media(current_time)?;

        let media = self.pool.lookup_media(&media_uuid).unwrap();

        let media_changed = match last_media_uuid {
            Some(ref last_media_uuid) => last_media_uuid != &media_uuid,
            None => true,
        };

        if !media_changed {
            self.used_tapes.insert(media_uuid.clone());
            return Ok(media_uuid);
        }

        task_log!(
            worker,
            "allocated new writable media '{}'",
            media.label_text()
        );

        if let Some(PoolWriterState { mut drive, .. }) = self.status.take() {
            if last_media_uuid.is_some() {
                task_log!(worker, "eject current media");
                drive.eject_media()?;
            }
        }

        let (drive_config, _digest) = pbs_config::drive::config()?;

        let (mut drive, old_media_id) = request_and_load_media(
            worker,
            &drive_config,
            &self.drive_name,
            media.label(),
            &self.notify_email,
        )?;

        // test for critical tape alert flags
        if let Ok(alert_flags) = drive.tape_alert_flags() {
            if !alert_flags.is_empty() {
                task_log!(worker, "TapeAlertFlags: {:?}", alert_flags);
                if tape_alert_flags_critical(alert_flags) {
                    self.pool.set_media_status_damaged(&media_uuid)?;
                    bail!(
                        "aborting due to critical tape alert flags: {:?}",
                        alert_flags
                    );
                }
            }
        }

        let (catalog, is_new_media) = update_media_set_label(
            worker,
            drive.as_mut(),
            old_media_id.media_set_label,
            media.id(),
        )?;

        self.catalog_set.lock().unwrap().append_catalog(catalog)?;

        let media_set = media.media_set_label().unwrap();

        let encrypt_fingerprint = media_set
            .encryption_key_fingerprint
            .clone()
            .map(|fp| (fp, media_set.uuid.clone()));

        drive.set_encryption(encrypt_fingerprint)?;

        self.status = Some(PoolWriterState {
            drive,
            media_uuid: media_uuid.clone(),
            at_eom: false,
            bytes_written: 0,
        });

        if is_new_media {
            // add catalogs from previous media
            self.append_media_set_catalogs(worker)?;
        }

        self.used_tapes.insert(media_uuid.clone());
        Ok(media_uuid)
    }

    fn open_catalog_file(uuid: &Uuid) -> Result<File, Error> {
        let mut path = PathBuf::from(TAPE_STATUS_DIR);
        path.push(uuid.to_string());
        path.set_extension("log");

        let file = std::fs::OpenOptions::new().read(true).open(&path)?;

        Ok(file)
    }

    // Check it tape is loaded, then move to EOM (if not already there)
    //
    // Returns the tape position at EOM.
    fn prepare_tape_write(status: &mut PoolWriterState, worker: &WorkerTask) -> Result<u64, Error> {
        if !status.at_eom {
            task_log!(worker, "moving to end of media");
            status.drive.move_to_eom(true)?;
            status.at_eom = true;
            task_log!(worker, "arrived at end of media");
        }

        let current_file_number = status.drive.current_file_number()?;
        if current_file_number < 2 {
            bail!(
                "got strange file position number from drive ({})",
                current_file_number
            );
        }

        Ok(current_file_number)
    }

    /// Move to EOM (if not already there), then write the current
    /// catalog to the tape. On success, this return 'Ok(true)'.

    /// Please note that this may fail when there is not enough space
    /// on the media (return value 'Ok(false, _)'). In that case, the
    /// archive is marked incomplete. The caller should mark the media
    /// as full and try again using another media.
    pub fn append_catalog_archive(&mut self, worker: &WorkerTask) -> Result<bool, Error> {
        let catalog_magic = self.catalog_version();

        let status = match self.status {
            Some(ref mut status) => status,
            None => bail!("PoolWriter - no media loaded"),
        };

        Self::prepare_tape_write(status, worker)?;

        let catalog_set = self.catalog_set.lock().unwrap();

        let catalog = match catalog_set.catalog {
            None => bail!("append_catalog_archive failed: no catalog - internal error"),
            Some(ref catalog) => catalog,
        };

        let media_set = self.pool.current_media_set();

        let media_list = media_set.media_list();
        let uuid = match media_list.last() {
            None => bail!("got empty media list - internal error"),
            Some(None) => bail!("got incomplete media list - internal error"),
            Some(Some(last_uuid)) => {
                if last_uuid != catalog.uuid() {
                    bail!("got wrong media - internal error");
                }
                last_uuid
            }
        };

        let seq_nr = media_list.len() - 1;

        let mut writer: Box<dyn TapeWrite> = status.drive.write_file()?;

        let mut file = Self::open_catalog_file(uuid)?;

        let done = tape_write_catalog(
            writer.as_mut(),
            uuid,
            media_set.uuid(),
            seq_nr,
            &mut file,
            catalog_magic,
        )?
        .is_some();

        Ok(done)
    }

    // Append catalogs for all previous media in set (without last)
    fn append_media_set_catalogs(&mut self, worker: &WorkerTask) -> Result<(), Error> {
        let media_set = self.pool.current_media_set();

        let mut media_list = media_set.media_list();
        if media_list.len() < 2 {
            return Ok(());
        }
        media_list = &media_list[..(media_list.len() - 1)];

        let catalog_magic = self.catalog_version();

        let status = match self.status {
            Some(ref mut status) => status,
            None => bail!("PoolWriter - no media loaded"),
        };

        Self::prepare_tape_write(status, worker)?;

        for (seq_nr, uuid) in media_list.iter().enumerate() {
            let uuid = match uuid {
                None => bail!("got incomplete media list - internal error"),
                Some(uuid) => uuid,
            };

            let mut writer: Box<dyn TapeWrite> = status.drive.write_file()?;

            let mut file = Self::open_catalog_file(uuid)?;

            task_log!(worker, "write catalog for previous media: {}", uuid);

            if tape_write_catalog(
                writer.as_mut(),
                uuid,
                media_set.uuid(),
                seq_nr,
                &mut file,
                catalog_magic,
            )?
            .is_none()
            {
                bail!("got EOM while writing start catalog");
            }
        }

        Ok(())
    }

    /// Move to EOM (if not already there), then creates a new snapshot
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
        worker: &WorkerTask,
        snapshot_reader: &SnapshotReader,
    ) -> Result<(bool, usize), Error> {
        let status = match self.status {
            Some(ref mut status) => status,
            None => bail!("PoolWriter - no media loaded"),
        };

        let current_file_number = Self::prepare_tape_write(status, worker)?;

        let (done, bytes_written) = {
            let mut writer: Box<dyn TapeWrite> = status.drive.write_file()?;

            match tape_write_snapshot_archive(writer.as_mut(), snapshot_reader)? {
                Some(content_uuid) => {
                    self.catalog_set.lock().unwrap().register_snapshot(
                        content_uuid,
                        current_file_number,
                        snapshot_reader.datastore_name(),
                        snapshot_reader.snapshot().backup_ns(),
                        snapshot_reader.snapshot().as_ref(),
                    )?;
                    (true, writer.bytes_written())
                }
                None => (false, writer.bytes_written()),
            }
        };

        status.bytes_written += bytes_written;

        let request_sync = status.bytes_written >= COMMIT_BLOCK_SIZE;

        if !done || request_sync {
            self.commit()?;
        }

        Ok((done, bytes_written))
    }

    /// Move to EOM (if not already there), then creates a new chunk
    /// archive and writes chunks from 'chunk_iter'. This stops when
    /// it detect LEOM or when we reach max archive size
    /// (4GB). Written chunks are registered in the media catalog.
    pub fn append_chunk_archive(
        &mut self,
        worker: &WorkerTask,
        chunk_iter: &mut std::iter::Peekable<NewChunksIterator>,
        store: &str,
    ) -> Result<(bool, usize), Error> {
        let status = match self.status {
            Some(ref mut status) => status,
            None => bail!("PoolWriter - no media loaded"),
        };

        let current_file_number = Self::prepare_tape_write(status, worker)?;

        let writer = status.drive.write_file()?;

        let start_time = SystemTime::now();

        let (saved_chunks, content_uuid, leom, bytes_written) =
            write_chunk_archive(worker, writer, chunk_iter, store, MAX_CHUNK_ARCHIVE_SIZE)?;

        status.bytes_written += bytes_written;

        let elapsed = start_time.elapsed()?.as_secs_f64();
        task_log!(
            worker,
            "wrote {} chunks ({:.2} MB at {:.2} MB/s)",
            saved_chunks.len(),
            bytes_written as f64 / 1_000_000.0,
            (bytes_written as f64) / (1_000_000.0 * elapsed),
        );

        let request_sync = status.bytes_written >= COMMIT_BLOCK_SIZE;

        // register chunks in media_catalog
        self.catalog_set.lock().unwrap().register_chunk_archive(
            content_uuid,
            current_file_number,
            store,
            &saved_chunks,
        )?;

        if leom || request_sync {
            self.commit()?;
        }

        Ok((leom, bytes_written))
    }

    pub fn spawn_chunk_reader_thread(
        &self,
        datastore: Arc<DataStore>,
        snapshot_reader: Arc<Mutex<SnapshotReader>>,
    ) -> Result<(std::thread::JoinHandle<()>, NewChunksIterator), Error> {
        NewChunksIterator::spawn(datastore, snapshot_reader, Arc::clone(&self.catalog_set))
    }

    pub(crate) fn catalog_version(&self) -> [u8; 8] {
        if self.ns_magic {
            PROXMOX_BACKUP_CATALOG_ARCHIVE_MAGIC_1_1
        } else {
            PROXMOX_BACKUP_CATALOG_ARCHIVE_MAGIC_1_0
        }
    }
}

/// write up to <max_size> of chunks
#[allow(clippy::type_complexity)]
fn write_chunk_archive<'a>(
    _worker: &WorkerTask,
    writer: Box<dyn 'a + TapeWrite>,
    chunk_iter: &mut std::iter::Peekable<NewChunksIterator>,
    store: &str,
    max_size: usize,
) -> Result<(Vec<[u8; 32]>, Uuid, bool, usize), Error> {
    let (mut writer, content_uuid) = ChunkArchiveWriter::new(writer, store, true)?;

    // we want to get the chunk list in correct order
    let mut chunk_list: Vec<[u8; 32]> = Vec::new();

    let mut leom = false;

    loop {
        let (digest, blob) = match chunk_iter.peek() {
            None => break,
            Some(Ok((digest, blob))) => (digest, blob),
            Some(Err(err)) => bail!("{}", err),
        };

        //println!("CHUNK {} size {}", hex::encode(digest), blob.raw_size());

        match writer.try_write_chunk(digest, blob) {
            Ok(true) => {
                chunk_list.push(*digest);
                chunk_iter.next(); // consume
            }
            Ok(false) => {
                // Note; we do not consume the chunk (no chunk_iter.next())
                leom = true;
                break;
            }
            Err(err) => bail!("write chunk failed - {}", err),
        }

        if writer.bytes_written() > max_size {
            //task_log!(worker, "Chunk Archive max size reached, closing archive");
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
) -> Result<(MediaCatalog, bool), Error> {
    let media_catalog;

    let new_set = match media_id.media_set_label {
        None => bail!("got media without media set - internal error"),
        Some(ref set) => set,
    };

    let key_config = if let Some(ref fingerprint) = new_set.encryption_key_fingerprint {
        let (config_map, _digest) = load_key_configs()?;
        match config_map.get(fingerprint) {
            Some(key_config) => Some(key_config.clone()),
            None => {
                bail!(
                    "unable to find tape encryption key config '{}'",
                    fingerprint
                );
            }
        }
    } else {
        None
    };

    let new_media = match old_set {
        None => {
            task_log!(worker, "writing new media set label");
            drive.write_media_set_label(new_set, key_config.as_ref())?;
            media_catalog = MediaCatalog::overwrite(TAPE_STATUS_DIR, media_id, false)?;
            true
        }
        Some(media_set_label) => {
            if new_set.uuid == media_set_label.uuid {
                if new_set.seq_nr != media_set_label.seq_nr {
                    bail!(
                        "got media with wrong media sequence number ({} != {}",
                        new_set.seq_nr,
                        media_set_label.seq_nr
                    );
                }
                if new_set.encryption_key_fingerprint != media_set_label.encryption_key_fingerprint
                {
                    bail!("detected changed encryption fingerprint - internal error");
                }
                media_catalog = MediaCatalog::open(TAPE_STATUS_DIR, media_id, true, false)?;

                // todo: verify last content/media_catalog somehow?

                false
            } else {
                task_log!(
                    worker,
                    "writing new media set label (overwrite '{}/{}')",
                    media_set_label.uuid.to_string(),
                    media_set_label.seq_nr,
                );

                drive.write_media_set_label(new_set, key_config.as_ref())?;
                media_catalog = MediaCatalog::overwrite(TAPE_STATUS_DIR, media_id, false)?;
                true
            }
        }
    };

    Ok((media_catalog, new_media))
}
