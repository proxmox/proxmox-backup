use anyhow::{bail, format_err, Error};
use nix::dir::Dir;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use ::serde::Serialize;
use serde_json::{json, Value};

use proxmox_router::{RpcEnvironment, RpcEnvironmentType};
use proxmox_sys::fs::{lock_dir_noblock_shared, replace_file, CreateOptions};

use pbs_api_types::Authid;
use pbs_datastore::backup_info::{BackupDir, BackupInfo};
use pbs_datastore::dynamic_index::DynamicIndexWriter;
use pbs_datastore::fixed_index::FixedIndexWriter;
use pbs_datastore::{DataBlob, DataStore};
use proxmox_rest_server::{formatter::*, WorkerTask};

use crate::backup::verify_backup_dir_with_lock;

use hyper::{Body, Response};

#[derive(Copy, Clone, Serialize)]
struct UploadStatistic {
    count: u64,
    size: u64,
    compressed_size: u64,
    duplicates: u64,
}

impl UploadStatistic {
    fn new() -> Self {
        Self {
            count: 0,
            size: 0,
            compressed_size: 0,
            duplicates: 0,
        }
    }
}

impl std::ops::Add for UploadStatistic {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self {
            count: self.count + other.count,
            size: self.size + other.size,
            compressed_size: self.compressed_size + other.compressed_size,
            duplicates: self.duplicates + other.duplicates,
        }
    }
}

struct DynamicWriterState {
    name: String,
    index: DynamicIndexWriter,
    offset: u64,
    chunk_count: u64,
    upload_stat: UploadStatistic,
}

struct FixedWriterState {
    name: String,
    index: FixedIndexWriter,
    size: usize,
    chunk_size: u32,
    chunk_count: u64,
    small_chunk_count: usize, // allow 0..1 small chunks (last chunk may be smaller)
    upload_stat: UploadStatistic,
    incremental: bool,
}

// key=digest, value=length
type KnownChunksMap = HashMap<[u8; 32], u32>;

struct SharedBackupState {
    finished: bool,
    uid_counter: usize,
    file_counter: usize, // successfully uploaded files
    dynamic_writers: HashMap<usize, DynamicWriterState>,
    fixed_writers: HashMap<usize, FixedWriterState>,
    known_chunks: KnownChunksMap,
    backup_size: u64, // sums up size of all files
    backup_stat: UploadStatistic,
}

impl SharedBackupState {
    // Raise error if finished flag is set
    fn ensure_unfinished(&self) -> Result<(), Error> {
        if self.finished {
            bail!("backup already marked as finished.");
        }
        Ok(())
    }

    // Get an unique integer ID
    pub fn next_uid(&mut self) -> usize {
        self.uid_counter += 1;
        self.uid_counter
    }
}

/// `RpcEnvironmet` implementation for backup service
#[derive(Clone)]
pub struct BackupEnvironment {
    env_type: RpcEnvironmentType,
    result_attributes: Value,
    auth_id: Authid,
    pub debug: bool,
    pub formatter: &'static dyn OutputFormatter,
    pub worker: Arc<WorkerTask>,
    pub datastore: Arc<DataStore>,
    pub backup_dir: BackupDir,
    pub last_backup: Option<BackupInfo>,
    state: Arc<Mutex<SharedBackupState>>,
}

impl BackupEnvironment {
    pub fn new(
        env_type: RpcEnvironmentType,
        auth_id: Authid,
        worker: Arc<WorkerTask>,
        datastore: Arc<DataStore>,
        backup_dir: BackupDir,
    ) -> Self {
        let state = SharedBackupState {
            finished: false,
            uid_counter: 0,
            file_counter: 0,
            dynamic_writers: HashMap::new(),
            fixed_writers: HashMap::new(),
            known_chunks: HashMap::new(),
            backup_size: 0,
            backup_stat: UploadStatistic::new(),
        };

        Self {
            result_attributes: json!({}),
            env_type,
            auth_id,
            worker,
            datastore,
            debug: false,
            formatter: JSON_FORMATTER,
            backup_dir,
            last_backup: None,
            state: Arc::new(Mutex::new(state)),
        }
    }

    /// Register a Chunk with associated length.
    ///
    /// We do not fully trust clients, so a client may only use registered
    /// chunks. Please use this method to register chunks from previous backups.
    pub fn register_chunk(&self, digest: [u8; 32], length: u32) -> Result<(), Error> {
        let mut state = self.state.lock().unwrap();

        state.ensure_unfinished()?;

        state.known_chunks.insert(digest, length);

        Ok(())
    }

    /// Register fixed length chunks after upload.
    ///
    /// Like `register_chunk()`, but additionally record statistics for
    /// the fixed index writer.
    pub fn register_fixed_chunk(
        &self,
        wid: usize,
        digest: [u8; 32],
        size: u32,
        compressed_size: u32,
        is_duplicate: bool,
    ) -> Result<(), Error> {
        let mut state = self.state.lock().unwrap();

        state.ensure_unfinished()?;

        let data = match state.fixed_writers.get_mut(&wid) {
            Some(data) => data,
            None => bail!("fixed writer '{}' not registered", wid),
        };

        if size > data.chunk_size {
            bail!(
                "fixed writer '{}' - got large chunk ({} > {}",
                data.name,
                size,
                data.chunk_size
            );
        }

        if size < data.chunk_size {
            data.small_chunk_count += 1;
            if data.small_chunk_count > 1 {
                bail!(
                    "fixed writer '{}' - detected multiple end chunks (chunk size too small)",
                    wid
                );
            }
        }

        // record statistics
        data.upload_stat.count += 1;
        data.upload_stat.size += size as u64;
        data.upload_stat.compressed_size += compressed_size as u64;
        if is_duplicate {
            data.upload_stat.duplicates += 1;
        }

        // register chunk
        state.known_chunks.insert(digest, size);

        Ok(())
    }

    /// Register dynamic length chunks after upload.
    ///
    /// Like `register_chunk()`, but additionally record statistics for
    /// the dynamic index writer.
    pub fn register_dynamic_chunk(
        &self,
        wid: usize,
        digest: [u8; 32],
        size: u32,
        compressed_size: u32,
        is_duplicate: bool,
    ) -> Result<(), Error> {
        let mut state = self.state.lock().unwrap();

        state.ensure_unfinished()?;

        let data = match state.dynamic_writers.get_mut(&wid) {
            Some(data) => data,
            None => bail!("dynamic writer '{}' not registered", wid),
        };

        // record statistics
        data.upload_stat.count += 1;
        data.upload_stat.size += size as u64;
        data.upload_stat.compressed_size += compressed_size as u64;
        if is_duplicate {
            data.upload_stat.duplicates += 1;
        }

        // register chunk
        state.known_chunks.insert(digest, size);

        Ok(())
    }

    pub fn lookup_chunk(&self, digest: &[u8; 32]) -> Option<u32> {
        let state = self.state.lock().unwrap();

        state.known_chunks.get(digest).copied()
    }

    /// Store the writer with an unique ID
    pub fn register_dynamic_writer(
        &self,
        index: DynamicIndexWriter,
        name: String,
    ) -> Result<usize, Error> {
        let mut state = self.state.lock().unwrap();

        state.ensure_unfinished()?;

        let uid = state.next_uid();

        state.dynamic_writers.insert(
            uid,
            DynamicWriterState {
                index,
                name,
                offset: 0,
                chunk_count: 0,
                upload_stat: UploadStatistic::new(),
            },
        );

        Ok(uid)
    }

    /// Store the writer with an unique ID
    pub fn register_fixed_writer(
        &self,
        index: FixedIndexWriter,
        name: String,
        size: usize,
        chunk_size: u32,
        incremental: bool,
    ) -> Result<usize, Error> {
        let mut state = self.state.lock().unwrap();

        state.ensure_unfinished()?;

        let uid = state.next_uid();

        state.fixed_writers.insert(
            uid,
            FixedWriterState {
                index,
                name,
                chunk_count: 0,
                size,
                chunk_size,
                small_chunk_count: 0,
                upload_stat: UploadStatistic::new(),
                incremental,
            },
        );

        Ok(uid)
    }

    /// Append chunk to dynamic writer
    pub fn dynamic_writer_append_chunk(
        &self,
        wid: usize,
        offset: u64,
        size: u32,
        digest: &[u8; 32],
    ) -> Result<(), Error> {
        let mut state = self.state.lock().unwrap();

        state.ensure_unfinished()?;

        let data = match state.dynamic_writers.get_mut(&wid) {
            Some(data) => data,
            None => bail!("dynamic writer '{}' not registered", wid),
        };

        if data.offset != offset {
            bail!(
                "dynamic writer '{}' append chunk failed - got strange chunk offset ({} != {})",
                data.name,
                data.offset,
                offset
            );
        }

        data.offset += size as u64;
        data.chunk_count += 1;

        data.index.add_chunk(data.offset, digest)?;

        Ok(())
    }

    /// Append chunk to fixed writer
    pub fn fixed_writer_append_chunk(
        &self,
        wid: usize,
        offset: u64,
        size: u32,
        digest: &[u8; 32],
    ) -> Result<(), Error> {
        let mut state = self.state.lock().unwrap();

        state.ensure_unfinished()?;

        let data = match state.fixed_writers.get_mut(&wid) {
            Some(data) => data,
            None => bail!("fixed writer '{}' not registered", wid),
        };

        let end = (offset as usize) + (size as usize);
        let idx = data.index.check_chunk_alignment(end, size as usize)?;

        data.chunk_count += 1;

        data.index.add_digest(idx, digest)?;

        Ok(())
    }

    fn log_upload_stat(
        &self,
        archive_name: &str,
        csum: &[u8; 32],
        uuid: &[u8; 16],
        size: u64,
        chunk_count: u64,
        upload_stat: &UploadStatistic,
    ) {
        self.log(format!("Upload statistics for '{}'", archive_name));
        self.log(format!("UUID: {}", hex::encode(uuid)));
        self.log(format!("Checksum: {}", hex::encode(csum)));
        self.log(format!("Size: {}", size));
        self.log(format!("Chunk count: {}", chunk_count));

        if size == 0 || chunk_count == 0 {
            return;
        }

        self.log(format!(
            "Upload size: {} ({}%)",
            upload_stat.size,
            (upload_stat.size * 100) / size
        ));

        // account for zero chunk, which might be uploaded but never used
        let client_side_duplicates = if chunk_count < upload_stat.count {
            0
        } else {
            chunk_count - upload_stat.count
        };

        let server_side_duplicates = upload_stat.duplicates;

        if (client_side_duplicates + server_side_duplicates) > 0 {
            let per = (client_side_duplicates + server_side_duplicates) * 100 / chunk_count;
            self.log(format!(
                "Duplicates: {}+{} ({}%)",
                client_side_duplicates, server_side_duplicates, per
            ));
        }

        if upload_stat.size > 0 {
            self.log(format!(
                "Compression: {}%",
                (upload_stat.compressed_size * 100) / upload_stat.size
            ));
        }
    }

    /// Close dynamic writer
    pub fn dynamic_writer_close(
        &self,
        wid: usize,
        chunk_count: u64,
        size: u64,
        csum: [u8; 32],
    ) -> Result<(), Error> {
        let mut state = self.state.lock().unwrap();

        state.ensure_unfinished()?;

        let mut data = match state.dynamic_writers.remove(&wid) {
            Some(data) => data,
            None => bail!("dynamic writer '{}' not registered", wid),
        };

        if data.chunk_count != chunk_count {
            bail!(
                "dynamic writer '{}' close failed - unexpected chunk count ({} != {})",
                data.name,
                data.chunk_count,
                chunk_count
            );
        }

        if data.offset != size {
            bail!(
                "dynamic writer '{}' close failed - unexpected file size ({} != {})",
                data.name,
                data.offset,
                size
            );
        }

        let uuid = data.index.uuid;

        let expected_csum = data.index.close()?;

        if csum != expected_csum {
            bail!(
                "dynamic writer '{}' close failed - got unexpected checksum",
                data.name
            );
        }

        self.log_upload_stat(
            &data.name,
            &csum,
            &uuid,
            size,
            chunk_count,
            &data.upload_stat,
        );

        state.file_counter += 1;
        state.backup_size += size;
        state.backup_stat = state.backup_stat + data.upload_stat;

        Ok(())
    }

    /// Close fixed writer
    pub fn fixed_writer_close(
        &self,
        wid: usize,
        chunk_count: u64,
        size: u64,
        csum: [u8; 32],
    ) -> Result<(), Error> {
        let mut state = self.state.lock().unwrap();

        state.ensure_unfinished()?;

        let mut data = match state.fixed_writers.remove(&wid) {
            Some(data) => data,
            None => bail!("fixed writer '{}' not registered", wid),
        };

        if data.chunk_count != chunk_count {
            bail!(
                "fixed writer '{}' close failed - received wrong number of chunk ({} != {})",
                data.name,
                data.chunk_count,
                chunk_count
            );
        }

        if !data.incremental {
            let expected_count = data.index.index_length();

            if chunk_count != (expected_count as u64) {
                bail!(
                    "fixed writer '{}' close failed - unexpected chunk count ({} != {})",
                    data.name,
                    expected_count,
                    chunk_count
                );
            }

            if size != (data.size as u64) {
                bail!(
                    "fixed writer '{}' close failed - unexpected file size ({} != {})",
                    data.name,
                    data.size,
                    size
                );
            }
        }

        let uuid = data.index.uuid;
        let expected_csum = data.index.close()?;

        if csum != expected_csum {
            bail!(
                "fixed writer '{}' close failed - got unexpected checksum",
                data.name
            );
        }

        self.log_upload_stat(
            &data.name,
            &expected_csum,
            &uuid,
            size,
            chunk_count,
            &data.upload_stat,
        );

        state.file_counter += 1;
        state.backup_size += size;
        state.backup_stat = state.backup_stat + data.upload_stat;

        Ok(())
    }

    pub fn add_blob(&self, file_name: &str, data: Vec<u8>) -> Result<(), Error> {
        let mut path = self.datastore.base_path();
        path.push(self.backup_dir.relative_path());
        path.push(file_name);

        let blob_len = data.len();
        let orig_len = data.len(); // fixme:

        // always verify blob/CRC at server side
        let blob = DataBlob::load_from_reader(&mut &data[..])?;

        let raw_data = blob.raw_data();
        replace_file(&path, raw_data, CreateOptions::new(), false)?;

        self.log(format!(
            "add blob {:?} ({} bytes, comp: {})",
            path, orig_len, blob_len
        ));

        let mut state = self.state.lock().unwrap();
        state.file_counter += 1;
        state.backup_size += orig_len as u64;
        state.backup_stat.size += blob_len as u64;

        Ok(())
    }

    /// Mark backup as finished
    pub fn finish_backup(&self) -> Result<(), Error> {
        let mut state = self.state.lock().unwrap();

        state.ensure_unfinished()?;

        // test if all writer are correctly closed
        if !state.dynamic_writers.is_empty() || !state.fixed_writers.is_empty() {
            bail!("found open index writer - unable to finish backup");
        }

        if state.file_counter == 0 {
            bail!("backup does not contain valid files (file count == 0)");
        }

        // check for valid manifest and store stats
        let stats = serde_json::to_value(state.backup_stat)?;
        self.backup_dir
            .update_manifest(|manifest| {
                manifest.unprotected["chunk_upload_stats"] = stats;
            })
            .map_err(|err| format_err!("unable to update manifest blob - {}", err))?;

        if let Some(base) = &self.last_backup {
            let path = base.backup_dir.full_path();
            if !path.exists() {
                bail!(
                    "base snapshot {} was removed during backup, cannot finish as chunks might be missing",
                    base.backup_dir.dir(),
                );
            }
        }

        self.datastore.try_ensure_sync_level()?;

        // marks the backup as successful
        state.finished = true;

        Ok(())
    }

    /// If verify-new is set on the datastore, this will run a new verify task
    /// for the backup. If not, this will return and also drop the passed lock
    /// immediately.
    pub fn verify_after_complete(&self, excl_snap_lock: Dir) -> Result<(), Error> {
        self.ensure_finished()?;

        if !self.datastore.verify_new() {
            // no verify requested, do nothing
            return Ok(());
        }

        // Downgrade to shared lock, the backup itself is finished
        drop(excl_snap_lock);
        let snap_lock = lock_dir_noblock_shared(
            &self.backup_dir.full_path(),
            "snapshot",
            "snapshot is already locked by another operation",
        )?;

        let worker_id = format!(
            "{}:{}/{}/{:08X}",
            self.datastore.name(),
            self.backup_dir.backup_type(),
            self.backup_dir.backup_id(),
            self.backup_dir.backup_time()
        );

        let datastore = self.datastore.clone();
        let backup_dir = self.backup_dir.clone();

        WorkerTask::new_thread(
            "verify",
            Some(worker_id),
            self.auth_id.to_string(),
            false,
            move |worker| {
                worker.log_message("Automatically verifying newly added snapshot");

                let verify_worker = crate::backup::VerifyWorker::new(worker.clone(), datastore);
                if !verify_backup_dir_with_lock(
                    &verify_worker,
                    &backup_dir,
                    worker.upid().clone(),
                    None,
                    snap_lock,
                )? {
                    bail!("verification failed - please check the log for details");
                }

                Ok(())
            },
        )
        .map(|_| ())
    }

    pub fn log<S: AsRef<str>>(&self, msg: S) {
        self.worker.log_message(msg);
    }

    pub fn debug<S: AsRef<str>>(&self, msg: S) {
        if self.debug {
            self.worker.log_message(msg);
        }
    }

    pub fn format_response(&self, result: Result<Value, Error>) -> Response<Body> {
        self.formatter.format_result(result, self)
    }

    /// Raise error if finished flag is not set
    pub fn ensure_finished(&self) -> Result<(), Error> {
        let state = self.state.lock().unwrap();
        if !state.finished {
            bail!("backup ended but finished flag is not set.");
        }
        Ok(())
    }

    /// Return true if the finished flag is set
    pub fn finished(&self) -> bool {
        let state = self.state.lock().unwrap();
        state.finished
    }

    /// Remove complete backup
    pub fn remove_backup(&self) -> Result<(), Error> {
        let mut state = self.state.lock().unwrap();
        state.finished = true;

        self.datastore.remove_backup_dir(
            self.backup_dir.backup_ns(),
            self.backup_dir.as_ref(),
            true,
        )?;

        Ok(())
    }
}

impl RpcEnvironment for BackupEnvironment {
    fn result_attrib_mut(&mut self) -> &mut Value {
        &mut self.result_attributes
    }

    fn result_attrib(&self) -> &Value {
        &self.result_attributes
    }

    fn env_type(&self) -> RpcEnvironmentType {
        self.env_type
    }

    fn set_auth_id(&mut self, _auth_id: Option<String>) {
        panic!("unable to change auth_id");
    }

    fn get_auth_id(&self) -> Option<String> {
        Some(self.auth_id.to_string())
    }
}

impl AsRef<BackupEnvironment> for dyn RpcEnvironment {
    fn as_ref(&self) -> &BackupEnvironment {
        self.as_any().downcast_ref::<BackupEnvironment>().unwrap()
    }
}

impl AsRef<BackupEnvironment> for Box<dyn RpcEnvironment> {
    fn as_ref(&self) -> &BackupEnvironment {
        self.as_any().downcast_ref::<BackupEnvironment>().unwrap()
    }
}
