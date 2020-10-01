use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{Ordering, AtomicUsize};
use std::time::Instant;

use anyhow::{bail, format_err, Error};

use crate::{
    server::WorkerTask,
    api2::types::*,
    tools::ParallelHandler,
    backup::{
        DataStore,
        DataBlob,
        BackupGroup,
        BackupDir,
        BackupInfo,
        IndexFile,
        CryptMode,
        FileInfo,
        ArchiveType,
        archive_type,
    },
};

fn verify_blob(datastore: Arc<DataStore>, backup_dir: &BackupDir, info: &FileInfo) -> Result<(), Error> {

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
    worker: Arc<WorkerTask>,
) {
    let (path, digest_str) = datastore.chunk_path(digest);

    let mut counter = 0;
    let mut new_path = path.clone();
    loop {
        new_path.set_file_name(format!("{}.{}.bad", digest_str, counter));
        if new_path.exists() && counter < 9 { counter += 1; } else { break; }
    }

    match std::fs::rename(&path, &new_path) {
        Ok(_) => {
            worker.log(format!("corrupted chunk renamed to {:?}", &new_path));
        },
        Err(err) => {
            match err.kind() {
                std::io::ErrorKind::NotFound => { /* ignored */ },
                _ => worker.log(format!("could not rename corrupted chunk {:?} - {}", &path, err))
            }
        }
    };
}

fn verify_index_chunks(
    datastore: Arc<DataStore>,
    index: Box<dyn IndexFile + Send>,
    verified_chunks: Arc<Mutex<HashSet<[u8;32]>>>,
    corrupt_chunks: Arc<Mutex<HashSet<[u8; 32]>>>,
    crypt_mode: CryptMode,
    worker: Arc<WorkerTask>,
) -> Result<(), Error> {

    let errors = Arc::new(AtomicUsize::new(0));

    let start_time = Instant::now();

    let mut read_bytes = 0;
    let mut decoded_bytes = 0;

    let worker2 = Arc::clone(&worker);
    let datastore2 = Arc::clone(&datastore);
    let corrupt_chunks2 = Arc::clone(&corrupt_chunks);
    let verified_chunks2 = Arc::clone(&verified_chunks);
    let errors2 = Arc::clone(&errors);

    let decoder_pool = ParallelHandler::new(
        "verify chunk decoder", 4,
        move |(chunk, digest, size): (DataBlob, [u8;32], u64)| {
            let chunk_crypt_mode = match chunk.crypt_mode() {
                Err(err) => {
                    corrupt_chunks2.lock().unwrap().insert(digest);
                    worker2.log(format!("can't verify chunk, unknown CryptMode - {}", err));
                    errors2.fetch_add(1, Ordering::SeqCst);
                    return Ok(());
                },
                Ok(mode) => mode,
            };

            if chunk_crypt_mode != crypt_mode {
                worker2.log(format!(
                    "chunk CryptMode {:?} does not match index CryptMode {:?}",
                    chunk_crypt_mode,
                    crypt_mode
                ));
                errors2.fetch_add(1, Ordering::SeqCst);
            }

            if let Err(err) = chunk.verify_unencrypted(size as usize, &digest) {
                corrupt_chunks2.lock().unwrap().insert(digest);
                worker2.log(format!("{}", err));
                errors2.fetch_add(1, Ordering::SeqCst);
                rename_corrupted_chunk(datastore2.clone(), &digest, worker2.clone());
            } else {
                verified_chunks2.lock().unwrap().insert(digest);
            }

            Ok(())
        }
    );

    for pos in 0..index.index_count() {

        worker.fail_on_abort()?;
        crate::tools::fail_on_shutdown()?;

        let info = index.chunk_info(pos).unwrap();
        let size = info.size();

        if verified_chunks.lock().unwrap().contains(&info.digest) {
            continue; // already verified
        }

        if corrupt_chunks.lock().unwrap().contains(&info.digest) {
            let digest_str = proxmox::tools::digest_to_hex(&info.digest);
            worker.log(format!("chunk {} was marked as corrupt", digest_str));
            errors.fetch_add(1, Ordering::SeqCst);
            continue;
        }

        match datastore.load_chunk(&info.digest) {
            Err(err) => {
                corrupt_chunks.lock().unwrap().insert(info.digest);
                worker.log(format!("can't verify chunk, load failed - {}", err));
                errors.fetch_add(1, Ordering::SeqCst);
                rename_corrupted_chunk(datastore.clone(), &info.digest, worker.clone());
                continue;
            }
            Ok(chunk) => {
                read_bytes += chunk.raw_size();
                decoder_pool.send((chunk, info.digest, size))?;
                decoded_bytes += size;
            }
        }
    }

    decoder_pool.complete()?;

    let elapsed = start_time.elapsed().as_secs_f64();

    let read_bytes_mib = (read_bytes as f64)/(1024.0*1024.0);
    let decoded_bytes_mib = (decoded_bytes as f64)/(1024.0*1024.0);

    let read_speed = read_bytes_mib/elapsed;
    let decode_speed = decoded_bytes_mib/elapsed;

    let error_count = errors.load(Ordering::SeqCst);

    worker.log(format!("  verified {:.2}/{:.2} MiB in {:.2} seconds, speed {:.2}/{:.2} MiB/s ({} errors)",
                       read_bytes_mib, decoded_bytes_mib, elapsed, read_speed, decode_speed, error_count));

    if errors.load(Ordering::SeqCst) > 0 {
        bail!("chunks could not be verified");
    }

    Ok(())
}

fn verify_fixed_index(
    datastore: Arc<DataStore>,
    backup_dir: &BackupDir,
    info: &FileInfo,
    verified_chunks: Arc<Mutex<HashSet<[u8;32]>>>,
    corrupt_chunks: Arc<Mutex<HashSet<[u8;32]>>>,
    worker: Arc<WorkerTask>,
) -> Result<(), Error> {

    let mut path = backup_dir.relative_path();
    path.push(&info.filename);

    let index = datastore.open_fixed_reader(&path)?;

    let (csum, size) = index.compute_csum();
    if size != info.size {
        bail!("wrong size ({} != {})", info.size, size);
    }

    if csum != info.csum {
        bail!("wrong index checksum");
    }

    verify_index_chunks(datastore, Box::new(index), verified_chunks, corrupt_chunks, info.chunk_crypt_mode(), worker)
}

fn verify_dynamic_index(
    datastore: Arc<DataStore>,
    backup_dir: &BackupDir,
    info: &FileInfo,
    verified_chunks: Arc<Mutex<HashSet<[u8;32]>>>,
    corrupt_chunks: Arc<Mutex<HashSet<[u8;32]>>>,
    worker: Arc<WorkerTask>,
) -> Result<(), Error> {

    let mut path = backup_dir.relative_path();
    path.push(&info.filename);

    let index = datastore.open_dynamic_reader(&path)?;

    let (csum, size) = index.compute_csum();
    if size != info.size {
        bail!("wrong size ({} != {})", info.size, size);
    }

    if csum != info.csum {
        bail!("wrong index checksum");
    }

    verify_index_chunks(datastore, Box::new(index), verified_chunks, corrupt_chunks, info.chunk_crypt_mode(), worker)
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
    datastore: Arc<DataStore>,
    backup_dir: &BackupDir,
    verified_chunks: Arc<Mutex<HashSet<[u8;32]>>>,
    corrupt_chunks: Arc<Mutex<HashSet<[u8;32]>>>,
    worker: Arc<WorkerTask>
) -> Result<bool, Error> {

    let mut manifest = match datastore.load_manifest(&backup_dir) {
        Ok((manifest, _)) => manifest,
        Err(err) => {
            worker.log(format!("verify {}:{} - manifest load error: {}", datastore.name(), backup_dir, err));
            return Ok(false);
        }
    };

    worker.log(format!("verify {}:{}", datastore.name(), backup_dir));

    let mut error_count = 0;

    let mut verify_result = VerifyState::Ok;
    for info in manifest.files() {
        let result = proxmox::try_block!({
            worker.log(format!("  check {}", info.filename));
            match archive_type(&info.filename)? {
                ArchiveType::FixedIndex =>
                    verify_fixed_index(
                        datastore.clone(),
                        &backup_dir,
                        info,
                        verified_chunks.clone(),
                        corrupt_chunks.clone(),
                        worker.clone(),
                    ),
                ArchiveType::DynamicIndex =>
                    verify_dynamic_index(
                        datastore.clone(),
                        &backup_dir,
                        info,
                        verified_chunks.clone(),
                        corrupt_chunks.clone(),
                        worker.clone(),
                    ),
                ArchiveType::Blob => verify_blob(datastore.clone(), &backup_dir, info),
            }
        });

        worker.fail_on_abort()?;
        crate::tools::fail_on_shutdown()?;

        if let Err(err) = result {
            worker.log(format!("verify {}:{}/{} failed: {}", datastore.name(), backup_dir, info.filename, err));
            error_count += 1;
            verify_result = VerifyState::Failed;
        }

    }

    let verify_state = SnapshotVerifyState {
        state: verify_result,
        upid: worker.upid().clone(),
    };
    manifest.unprotected["verify_state"] = serde_json::to_value(verify_state)?;
    datastore.store_manifest(&backup_dir, serde_json::to_value(manifest)?)
        .map_err(|err| format_err!("unable to store manifest blob - {}", err))?;

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
    datastore: Arc<DataStore>,
    group: &BackupGroup,
    verified_chunks: Arc<Mutex<HashSet<[u8;32]>>>,
    corrupt_chunks: Arc<Mutex<HashSet<[u8;32]>>>,
    progress: Option<(usize, usize)>, // (done, snapshot_count)
    worker: Arc<WorkerTask>,
) -> Result<(usize, Vec<String>), Error> {

    let mut errors = Vec::new();
    let mut list = match group.list_backups(&datastore.base_path()) {
        Ok(list) => list,
        Err(err) => {
            worker.log(format!("verify group {}:{} - unable to list backups: {}", datastore.name(), group, err));
            return Ok((0, errors));
        }
    };

    worker.log(format!("verify group {}:{}", datastore.name(), group));

    let (done, snapshot_count) = progress.unwrap_or((0, list.len()));

    let mut count = 0;
    BackupInfo::sort_list(&mut list, false); // newest first
    for info in list {
        count += 1;
        if !verify_backup_dir(datastore.clone(), &info.backup_dir, verified_chunks.clone(), corrupt_chunks.clone(), worker.clone())?{
            errors.push(info.backup_dir.to_string());
        }
        if snapshot_count != 0 {
            let pos = done + count;
            let percentage = ((pos as f64) * 100.0)/(snapshot_count as f64);
            worker.log(format!("percentage done: {:.2}% ({} of {} snapshots)", percentage, pos, snapshot_count));
        }
    }

    Ok((count, errors))
}

/// Verify all backups inside a datastore
///
/// Errors are logged to the worker log.
///
/// Returns
/// - Ok(failed_dirs) where failed_dirs had verification errors
/// - Err(_) if task was aborted
pub fn verify_all_backups(datastore: Arc<DataStore>, worker: Arc<WorkerTask>) -> Result<Vec<String>, Error> {

    let mut errors = Vec::new();

    let mut list = match BackupGroup::list_groups(&datastore.base_path()) {
        Ok(list) => list
            .into_iter()
            .filter(|group| !(group.backup_type() == "host" && group.backup_id() == "benchmark"))
            .collect::<Vec<BackupGroup>>(),
        Err(err) => {
            worker.log(format!("verify datastore {} - unable to list backups: {}", datastore.name(), err));
            return Ok(errors);
        }
    };

    list.sort_unstable();

    let mut snapshot_count = 0;
    for group in list.iter() {
        snapshot_count += group.list_backups(&datastore.base_path())?.len();
    }

    // start with 16384 chunks (up to 65GB)
    let verified_chunks = Arc::new(Mutex::new(HashSet::with_capacity(1024*16)));

    // start with 64 chunks since we assume there are few corrupt ones
    let corrupt_chunks = Arc::new(Mutex::new(HashSet::with_capacity(64)));

    worker.log(format!("verify datastore {} ({} snapshots)", datastore.name(), snapshot_count));

    let mut done = 0;
    for group in list {
        let (count, mut group_errors) = verify_backup_group(
            datastore.clone(),
            &group,
            verified_chunks.clone(),
            corrupt_chunks.clone(),
            Some((done, snapshot_count)),
            worker.clone(),
        )?;
        errors.append(&mut group_errors);

        done += count;
    }

    Ok(errors)
}
