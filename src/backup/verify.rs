use std::collections::HashSet;

use anyhow::{bail, Error};

use crate::server::WorkerTask;

use super::{
    DataStore, BackupGroup, BackupDir, BackupInfo, IndexFile,
    ENCR_COMPR_BLOB_MAGIC_1_0, ENCRYPTED_BLOB_MAGIC_1_0,
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

    let magic = blob.magic();

    if magic == &ENCR_COMPR_BLOB_MAGIC_1_0 || magic == &ENCRYPTED_BLOB_MAGIC_1_0 {
        return Ok(());
    }

    blob.decode(None)?;

    Ok(())
}

fn verify_index_chunks(
    datastore: &DataStore,
    index: Box<dyn IndexFile>,
    verified_chunks: &mut HashSet<[u8;32]>,
    corrupt_chunks: &mut HashSet<[u8; 32]>,
    worker: &WorkerTask,
) -> Result<(), Error> {

    let mut errors = 0;
    for pos in 0..index.index_count() {

        worker.fail_on_abort()?;

        let info = index.chunk_info(pos).unwrap();
        let size = info.range.end - info.range.start;

        if !verified_chunks.contains(&info.digest) {
            if !corrupt_chunks.contains(&info.digest) {
                if let Err(err) = datastore.verify_stored_chunk(&info.digest, size) {
                    corrupt_chunks.insert(info.digest);
                    worker.log(format!("{}", err));
                    errors += 1;
                } else {
                    verified_chunks.insert(info.digest);
                }
            } else {
                let digest_str = proxmox::tools::digest_to_hex(&info.digest);
                worker.log(format!("chunk {} was marked as corrupt", digest_str));
                errors += 1;
            }
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

    verify_index_chunks(datastore, Box::new(index), verified_chunks, corrupt_chunks, worker)
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

    verify_index_chunks(datastore, Box::new(index), verified_chunks, corrupt_chunks, worker)
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

    let list = match BackupGroup::list_groups(&datastore.base_path()) {
        Ok(list) => list,
        Err(err) => {
            worker.log(format!("verify datastore {} - unable to list backups: {}", datastore.name(), err));
            return Ok(errors);
        }
    };

    worker.log(format!("verify datastore {}", datastore.name()));

    for group in list {
        let mut group_errors = verify_backup_group(datastore, &group, worker)?;
        errors.append(&mut group_errors);
    }

    Ok(errors)
}
