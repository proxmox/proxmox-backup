use std::collections::HashSet;

use anyhow::{bail, Error};

use crate::server::WorkerTask;

use super::{
    DataStore, BackupGroup, BackupDir, BackupInfo, IndexFile,
    CryptMode,
    FileInfo, ArchiveType, archive_type,
};

fn verify_blob(datastore: &DataStore, backup_dir: &BackupDir, info: &FileInfo) -> Result<(), Error> {

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

fn verify_index_chunks(
    datastore: &DataStore,
    index: Box<dyn IndexFile>,
    verified_chunks: &mut HashSet<[u8;32]>,
    corrupt_chunks: &mut HashSet<[u8; 32]>,
    crypt_mode: CryptMode,
    worker: &WorkerTask,
) -> Result<(), Error> {

    let mut errors = 0;
    for pos in 0..index.index_count() {

        worker.fail_on_abort()?;

        let info = index.chunk_info(pos).unwrap();

        if verified_chunks.contains(&info.digest) {
            continue; // already verified
        }

        if corrupt_chunks.contains(&info.digest) {
            let digest_str = proxmox::tools::digest_to_hex(&info.digest);
            worker.log(format!("chunk {} was marked as corrupt", digest_str));
            errors += 1;
            continue;
        }

        let chunk = match datastore.load_chunk(&info.digest) {
            Err(err) => {
                corrupt_chunks.insert(info.digest);
                worker.log(format!("can't verify chunk, load failed - {}", err));
                errors += 1;
                continue;
            },
            Ok(chunk) => chunk,
        };

        let chunk_crypt_mode = match chunk.crypt_mode() {
            Err(err) => {
                corrupt_chunks.insert(info.digest);
                worker.log(format!("can't verify chunk, unknown CryptMode - {}", err));
                errors += 1;
                continue;
            },
            Ok(mode) => mode,
        };

        if chunk_crypt_mode != crypt_mode {
            worker.log(format!(
                "chunk CryptMode {:?} does not match index CryptMode {:?}",
                chunk_crypt_mode,
                crypt_mode
            ));
            errors += 1;
        }

        let size = info.range.end - info.range.start;

        if let Err(err) = chunk.verify_unencrypted(size as usize, &info.digest) {
            corrupt_chunks.insert(info.digest);
            worker.log(format!("{}", err));
            errors += 1;
        } else {
            verified_chunks.insert(info.digest);
        }
    }

    if errors > 0 {
        bail!("chunks could not be verified");
    }

    Ok(())
}

fn verify_fixed_index(
    datastore: &DataStore,
    backup_dir: &BackupDir,
    info: &FileInfo,
    verified_chunks: &mut HashSet<[u8;32]>,
    corrupt_chunks: &mut HashSet<[u8;32]>,
    worker: &WorkerTask,
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
    datastore: &DataStore,
    backup_dir: &BackupDir,
    info: &FileInfo,
    verified_chunks: &mut HashSet<[u8;32]>,
    corrupt_chunks: &mut HashSet<[u8;32]>,
    worker: &WorkerTask,
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
    datastore: &DataStore,
    backup_dir: &BackupDir,
    verified_chunks: &mut HashSet<[u8;32]>,
    corrupt_chunks: &mut HashSet<[u8;32]>,
    worker: &WorkerTask
) -> Result<bool, Error> {

    let manifest = match datastore.load_manifest(&backup_dir) {
        Ok((manifest, _)) => manifest,
        Err(err) => {
            worker.log(format!("verify {}:{} - manifest load error: {}", datastore.name(), backup_dir, err));
            return Ok(false);
        }
    };

    worker.log(format!("verify {}:{}", datastore.name(), backup_dir));

    let mut error_count = 0;

    for info in manifest.files() {
        let result = proxmox::try_block!({
            worker.log(format!("  check {}", info.filename));
            match archive_type(&info.filename)? {
                ArchiveType::FixedIndex =>
                    verify_fixed_index(
                        &datastore,
                        &backup_dir,
                        info,
                        verified_chunks,
                        corrupt_chunks,
                        worker
                    ),
                ArchiveType::DynamicIndex =>
                    verify_dynamic_index(
                        &datastore,
                        &backup_dir,
                        info,
                        verified_chunks,
                        corrupt_chunks,
                        worker
                    ),
                ArchiveType::Blob => verify_blob(&datastore, &backup_dir, info),
            }
        });

        worker.fail_on_abort()?;

        if let Err(err) = result {
            worker.log(format!("verify {}:{}/{} failed: {}", datastore.name(), backup_dir, info.filename, err));
            error_count += 1;
        }
    }

    Ok(error_count == 0)
}

/// Verify all backups inside a backup group
///
/// Errors are logged to the worker log.
///
/// Returns
/// - Ok(failed_dirs) where failed_dirs had verification errors
/// - Err(_) if task was aborted
pub fn verify_backup_group(datastore: &DataStore, group: &BackupGroup, worker: &WorkerTask) -> Result<Vec<String>, Error> {

    let mut errors = Vec::new();
    let mut list = match group.list_backups(&datastore.base_path()) {
        Ok(list) => list,
        Err(err) => {
            worker.log(format!("verify group {}:{} - unable to list backups: {}", datastore.name(), group, err));
            return Ok(errors);
        }
    };

    worker.log(format!("verify group {}:{}", datastore.name(), group));

    let mut verified_chunks = HashSet::with_capacity(1024*16); // start with 16384 chunks (up to 65GB)
    let mut corrupt_chunks = HashSet::with_capacity(64); // start with 64 chunks since we assume there are few corrupt ones

    BackupInfo::sort_list(&mut list, false); // newest first
    for info in list {
        if !verify_backup_dir(datastore, &info.backup_dir, &mut verified_chunks, &mut corrupt_chunks, worker)?{
            errors.push(info.backup_dir.to_string());
        }
    }

    Ok(errors)
}

/// Verify all backups inside a datastore
///
/// Errors are logged to the worker log.
///
/// Returns
/// - Ok(failed_dirs) where failed_dirs had verification errors
/// - Err(_) if task was aborted
pub fn verify_all_backups(datastore: &DataStore, worker: &WorkerTask) -> Result<Vec<String>, Error> {

    let mut errors = Vec::new();

    let mut list = match BackupGroup::list_groups(&datastore.base_path()) {
        Ok(list) => list,
        Err(err) => {
            worker.log(format!("verify datastore {} - unable to list backups: {}", datastore.name(), err));
            return Ok(errors);
        }
    };

    list.sort_unstable();

    worker.log(format!("verify datastore {}", datastore.name()));

    for group in list {
        let mut group_errors = verify_backup_group(datastore, &group, worker)?;
        errors.append(&mut group_errors);
    }

    Ok(errors)
}
