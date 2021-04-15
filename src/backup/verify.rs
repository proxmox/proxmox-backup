use nix::dir::Dir;
use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::{bail, format_err, Error};

use crate::{
    api2::types::*,
    backup::{
        DataStore,
        StoreProgress,
        DataBlob,
        BackupGroup,
        BackupDir,
        BackupInfo,
        BackupManifest,
        IndexFile,
        CryptMode,
        FileInfo,
        ArchiveType,
        archive_type,
    },
    server::UPID,
    task::TaskState,
    task_log,
    tools::fs::lock_dir_noblock_shared,
    tools::ParallelHandler,
};

/// A VerifyWorker encapsulates a task worker, datastore and information about which chunks have
/// already been verified or detected as corrupt.
pub struct VerifyWorker {
    worker: Arc<dyn TaskState + Send + Sync>,
    datastore: Arc<DataStore>,
    verified_chunks: Arc<Mutex<HashSet<[u8; 32]>>>,
    corrupt_chunks: Arc<Mutex<HashSet<[u8; 32]>>>,
}

impl VerifyWorker {
    /// Creates a new VerifyWorker for a given task worker and datastore.
    pub fn new(worker: Arc<dyn TaskState + Send + Sync>, datastore: Arc<DataStore>) -> Self {
        Self {
            worker,
            datastore,
            // start with 16k chunks == up to 64G data
            verified_chunks: Arc::new(Mutex::new(HashSet::with_capacity(16 * 1024))),
            // start with 64 chunks since we assume there are few corrupt ones
            corrupt_chunks: Arc::new(Mutex::new(HashSet::with_capacity(64))),
        }
    }
}

fn verify_blob(
    datastore: Arc<DataStore>,
    backup_dir: &BackupDir,
    info: &FileInfo,
) -> Result<(), Error> {
    let blob = datastore.load_blob(backup_dir, &info.filename)?;

    let raw_size = blob.raw_size();
    if raw_size != info.size {
        bail!("wrong size ({} != {})", info.size, raw_size);
    }

    let csum = openssl::sha::sha256(blob.raw_data());
    if csum != info.csum {
        bail!("wrong index checksum");
    }

    match blob.crypt_mode()? {
        CryptMode::Encrypt => Ok(()),
        CryptMode::None => {
            // digest already verified above
            blob.decode(None, None)?;
            Ok(())
        },
        CryptMode::SignOnly => bail!("Invalid CryptMode for blob"),
    }
}

fn rename_corrupted_chunk(
    datastore: Arc<DataStore>,
    digest: &[u8;32],
    worker: &dyn TaskState,
) {
    let (path, digest_str) = datastore.chunk_path(digest);

    let mut counter = 0;
    let mut new_path = path.clone();
    loop {
        new_path.set_file_name(format!("{}.{}.bad", digest_str, counter));
        if new_path.exists() && counter < 9 {
            counter += 1;
        } else {
            break;
        }
    }

    match std::fs::rename(&path, &new_path) {
        Ok(_) => {
            task_log!(worker, "corrupted chunk renamed to {:?}", &new_path);
        },
        Err(err) => {
            match err.kind() {
                std::io::ErrorKind::NotFound => { /* ignored */ },
                _ => task_log!(worker, "could not rename corrupted chunk {:?} - {}", &path, err)
            }
        }
    };
}

fn verify_index_chunks(
    verify_worker: &VerifyWorker,
    index: Box<dyn IndexFile + Send>,
    crypt_mode: CryptMode,
) -> Result<(), Error> {
    let errors = Arc::new(AtomicUsize::new(0));

    let start_time = Instant::now();

    let mut read_bytes = 0;
    let mut decoded_bytes = 0;

    let worker2 = Arc::clone(&verify_worker.worker);
    let datastore2 = Arc::clone(&verify_worker.datastore);
    let corrupt_chunks2 = Arc::clone(&verify_worker.corrupt_chunks);
    let verified_chunks2 = Arc::clone(&verify_worker.verified_chunks);
    let errors2 = Arc::clone(&errors);

    let decoder_pool = ParallelHandler::new(
        "verify chunk decoder",
        4,
        move |(chunk, digest, size): (DataBlob, [u8; 32], u64)| {
            let chunk_crypt_mode = match chunk.crypt_mode() {
                Err(err) => {
                    corrupt_chunks2.lock().unwrap().insert(digest);
                    task_log!(worker2, "can't verify chunk, unknown CryptMode - {}", err);
                    errors2.fetch_add(1, Ordering::SeqCst);
                    return Ok(());
                },
                Ok(mode) => mode,
            };

            if chunk_crypt_mode != crypt_mode {
                task_log!(
                    worker2,
                    "chunk CryptMode {:?} does not match index CryptMode {:?}",
                    chunk_crypt_mode,
                    crypt_mode
                );
                errors2.fetch_add(1, Ordering::SeqCst);
            }

            if let Err(err) = chunk.verify_unencrypted(size as usize, &digest) {
                corrupt_chunks2.lock().unwrap().insert(digest);
                task_log!(worker2, "{}", err);
                errors2.fetch_add(1, Ordering::SeqCst);
                rename_corrupted_chunk(datastore2.clone(), &digest, &worker2);
            } else {
                verified_chunks2.lock().unwrap().insert(digest);
            }

            Ok(())
        }
    );

    let index_count = index.index_count();
    let mut chunk_list = Vec::with_capacity(index_count);

    use std::os::unix::fs::MetadataExt;

    for pos in 0..index_count {
        verify_worker.worker.check_abort()?;
        crate::tools::fail_on_shutdown()?;

        let info = index.chunk_info(pos).unwrap();

        if verify_worker.verified_chunks.lock().unwrap().contains(&info.digest) {
            continue; // already verified
        }

        if verify_worker.corrupt_chunks.lock().unwrap().contains(&info.digest) {
            let digest_str = proxmox::tools::digest_to_hex(&info.digest);
            task_log!(verify_worker.worker, "chunk {} was marked as corrupt", digest_str);
            errors.fetch_add(1, Ordering::SeqCst);
            continue;
        }

        match verify_worker.datastore.stat_chunk(&info.digest) {
            Err(err) => {
                verify_worker.corrupt_chunks.lock().unwrap().insert(info.digest);
                task_log!(verify_worker.worker, "can't verify chunk, stat failed - {}", err);
                errors.fetch_add(1, Ordering::SeqCst);
                rename_corrupted_chunk(
                    verify_worker.datastore.clone(),
                    &info.digest,
                    &verify_worker.worker,
                );
            }
            Ok(metadata) => {
                chunk_list.push((pos, metadata.ino()));
            }
        }
    }

    // sorting by inode improves data locality, which makes it lots faster on spinners
    chunk_list.sort_unstable_by(|(_, ino_a), (_, ino_b)| ino_a.cmp(&ino_b));

    for (pos, _) in chunk_list {
        verify_worker.worker.check_abort()?;
        crate::tools::fail_on_shutdown()?;

        let info = index.chunk_info(pos).unwrap();

        // we must always recheck this here, the parallel worker below alter it!
        // Else we miss skipping repeated chunks from the same index, and re-verify them all
        if verify_worker.verified_chunks.lock().unwrap().contains(&info.digest) {
            continue; // already verified
        }

        match verify_worker.datastore.load_chunk(&info.digest) {
            Err(err) => {
                verify_worker.corrupt_chunks.lock().unwrap().insert(info.digest);
                task_log!(verify_worker.worker, "can't verify chunk, load failed - {}", err);
                errors.fetch_add(1, Ordering::SeqCst);
                rename_corrupted_chunk(
                    verify_worker.datastore.clone(),
                    &info.digest,
                    &verify_worker.worker,
                );
            }
            Ok(chunk) => {
                let size = info.size();
                read_bytes += chunk.raw_size();
                decoder_pool.send((chunk, info.digest, size))?;
                decoded_bytes += size;
            }
        }
    }

    decoder_pool.complete()?;

    let elapsed = start_time.elapsed().as_secs_f64();

    let read_bytes_mib = (read_bytes as f64) / (1024.0 * 1024.0);
    let decoded_bytes_mib = (decoded_bytes as f64) / (1024.0 * 1024.0);

    let read_speed = read_bytes_mib / elapsed;
    let decode_speed = decoded_bytes_mib / elapsed;

    let error_count = errors.load(Ordering::SeqCst);

    task_log!(
        verify_worker.worker,
        "  verified {:.2}/{:.2} MiB in {:.2} seconds, speed {:.2}/{:.2} MiB/s ({} errors)",
        read_bytes_mib,
        decoded_bytes_mib,
        elapsed,
        read_speed,
        decode_speed,
        error_count,
    );

    if errors.load(Ordering::SeqCst) > 0 {
        bail!("chunks could not be verified");
    }

    Ok(())
}

fn verify_fixed_index(
    verify_worker: &VerifyWorker,
    backup_dir: &BackupDir,
    info: &FileInfo,
) -> Result<(), Error> {
    let mut path = backup_dir.relative_path();
    path.push(&info.filename);

    let index = verify_worker.datastore.open_fixed_reader(&path)?;

    let (csum, size) = index.compute_csum();
    if size != info.size {
        bail!("wrong size ({} != {})", info.size, size);
    }

    if csum != info.csum {
        bail!("wrong index checksum");
    }

    verify_index_chunks(verify_worker, Box::new(index), info.chunk_crypt_mode())
}

fn verify_dynamic_index(
    verify_worker: &VerifyWorker,
    backup_dir: &BackupDir,
    info: &FileInfo,
) -> Result<(), Error> {
    let mut path = backup_dir.relative_path();
    path.push(&info.filename);

    let index = verify_worker.datastore.open_dynamic_reader(&path)?;

    let (csum, size) = index.compute_csum();
    if size != info.size {
        bail!("wrong size ({} != {})", info.size, size);
    }

    if csum != info.csum {
        bail!("wrong index checksum");
    }

    verify_index_chunks(verify_worker, Box::new(index), info.chunk_crypt_mode())
}

/// Verify a single backup snapshot
///
/// This checks all archives inside a backup snapshot.
/// Errors are logged to the worker log.
///
/// Returns
/// - Ok(true) if verify is successful
/// - Ok(false) if there were verification errors
/// - Err(_) if task was aborted
pub fn verify_backup_dir(
    verify_worker: &VerifyWorker,
    backup_dir: &BackupDir,
    upid: UPID,
    filter: Option<&dyn Fn(&BackupManifest) -> bool>,
) -> Result<bool, Error> {
    let snap_lock = lock_dir_noblock_shared(
        &verify_worker.datastore.snapshot_path(&backup_dir),
        "snapshot",
        "locked by another operation",
    );
    match snap_lock {
        Ok(snap_lock) => {
            verify_backup_dir_with_lock(verify_worker, backup_dir, upid, filter, snap_lock)
        }
        Err(err) => {
            task_log!(
                verify_worker.worker,
                "SKIPPED: verify {}:{} - could not acquire snapshot lock: {}",
                verify_worker.datastore.name(),
                backup_dir,
                err,
            );
            Ok(true)
        }
    }
}

/// See verify_backup_dir
pub fn verify_backup_dir_with_lock(
    verify_worker: &VerifyWorker,
    backup_dir: &BackupDir,
    upid: UPID,
    filter: Option<&dyn Fn(&BackupManifest) -> bool>,
    _snap_lock: Dir,
) -> Result<bool, Error> {
    let manifest = match verify_worker.datastore.load_manifest(&backup_dir) {
        Ok((manifest, _)) => manifest,
        Err(err) => {
            task_log!(
                verify_worker.worker,
                "verify {}:{} - manifest load error: {}",
                verify_worker.datastore.name(),
                backup_dir,
                err,
            );
            return Ok(false);
        }
    };

    if let Some(filter) = filter {
        if !filter(&manifest) {
            task_log!(
                verify_worker.worker,
                "SKIPPED: verify {}:{} (recently verified)",
                verify_worker.datastore.name(),
                backup_dir,
            );
            return Ok(true);
        }
    }

    task_log!(verify_worker.worker, "verify {}:{}", verify_worker.datastore.name(), backup_dir);

    let mut error_count = 0;

    let mut verify_result = VerifyState::Ok;
    for info in manifest.files() {
        let result = proxmox::try_block!({
            task_log!(verify_worker.worker, "  check {}", info.filename);
            match archive_type(&info.filename)? {
                ArchiveType::FixedIndex => verify_fixed_index(verify_worker, &backup_dir, info),
                ArchiveType::DynamicIndex => verify_dynamic_index(verify_worker, &backup_dir, info),
                ArchiveType::Blob => {
                    verify_blob(verify_worker.datastore.clone(), &backup_dir, info)
                }
            }
        });

        verify_worker.worker.check_abort()?;
        crate::tools::fail_on_shutdown()?;

        if let Err(err) = result {
            task_log!(
                verify_worker.worker,
                "verify {}:{}/{} failed: {}",
                verify_worker.datastore.name(),
                backup_dir,
                info.filename,
                err,
            );
            error_count += 1;
            verify_result = VerifyState::Failed;
        }
    }

    let verify_state = SnapshotVerifyState {
        state: verify_result,
        upid,
    };
    let verify_state = serde_json::to_value(verify_state)?;
    verify_worker
        .datastore
        .update_manifest(&backup_dir, |manifest| {
            manifest.unprotected["verify_state"] = verify_state;
        })
        .map_err(|err| format_err!("unable to update manifest blob - {}", err))?;

    Ok(error_count == 0)
}

/// Verify all backups inside a backup group
///
/// Errors are logged to the worker log.
///
/// Returns
/// - Ok((count, failed_dirs)) where failed_dirs had verification errors
/// - Err(_) if task was aborted
pub fn verify_backup_group(
    verify_worker: &VerifyWorker,
    group: &BackupGroup,
    progress: &mut StoreProgress,
    upid: &UPID,
    filter: Option<&dyn Fn(&BackupManifest) -> bool>,
) -> Result<Vec<String>, Error> {
    let mut errors = Vec::new();
    let mut list = match group.list_backups(&verify_worker.datastore.base_path()) {
        Ok(list) => list,
        Err(err) => {
            task_log!(
                verify_worker.worker,
                "verify group {}:{} - unable to list backups: {}",
                verify_worker.datastore.name(),
                group,
                err,
            );
            return Ok(errors);
        }
    };

    let snapshot_count = list.len();
    task_log!(
        verify_worker.worker,
        "verify group {}:{} ({} snapshots)",
        verify_worker.datastore.name(),
        group,
        snapshot_count
    );

    progress.group_snapshots = snapshot_count as u64;

    BackupInfo::sort_list(&mut list, false); // newest first
    for (pos, info) in list.into_iter().enumerate() {
        if !verify_backup_dir(verify_worker, &info.backup_dir, upid.clone(), filter)? {
            errors.push(info.backup_dir.to_string());
        }
        progress.done_snapshots = pos as u64 + 1;
        task_log!(verify_worker.worker, "percentage done: {}", progress);
    }

    Ok(errors)
}

/// Verify all (owned) backups inside a datastore
///
/// Errors are logged to the worker log.
///
/// Returns
/// - Ok(failed_dirs) where failed_dirs had verification errors
/// - Err(_) if task was aborted
pub fn verify_all_backups(
    verify_worker: &VerifyWorker,
    upid: &UPID,
    owner: Option<Authid>,
    filter: Option<&dyn Fn(&BackupManifest) -> bool>,
) -> Result<Vec<String>, Error> {
    let mut errors = Vec::new();
    let worker = Arc::clone(&verify_worker.worker);

    task_log!(worker, "verify datastore {}", verify_worker.datastore.name());

    if let Some(owner) = &owner {
        task_log!(worker, "limiting to backups owned by {}", owner);
    }

    let filter_by_owner = |group: &BackupGroup| {
        match (verify_worker.datastore.get_owner(group), &owner) {
            (Ok(ref group_owner), Some(owner)) => {
                group_owner == owner
                    || (group_owner.is_token()
                        && !owner.is_token()
                        && group_owner.user() == owner.user())
            },
            (Ok(_), None) => true,
            (Err(err), Some(_)) => {
                // intentionally not in task log
                // the task user might not be allowed to see this group!
                println!("Failed to get owner of group '{}' - {}", group, err);
                false
            },
            (Err(err), None) => {
                // we don't filter by owner, but we want to log the error
                task_log!(
                    worker,
                    "Failed to get owner of group '{} - {}",
                    group,
                    err,
                );
                errors.push(group.to_string());
                true
            },
        }
    };

    let mut list = match BackupInfo::list_backup_groups(&verify_worker.datastore.base_path()) {
        Ok(list) => list
            .into_iter()
            .filter(|group| !(group.backup_type() == "host" && group.backup_id() == "benchmark"))
            .filter(filter_by_owner)
            .collect::<Vec<BackupGroup>>(),
        Err(err) => {
            task_log!(worker, "unable to list backups: {}", err,);
            return Ok(errors);
        }
    };

    list.sort_unstable();

    let group_count = list.len();
    task_log!(worker, "found {} groups", group_count);

    let mut progress = StoreProgress::new(group_count as u64);

    for (pos, group) in list.into_iter().enumerate() {
        progress.done_groups = pos as u64;
        progress.done_snapshots = 0;
        progress.group_snapshots = 0;

        let mut group_errors =
            verify_backup_group(verify_worker, &group, &mut progress, upid, filter)?;
        errors.append(&mut group_errors);
    }

    Ok(errors)
}
