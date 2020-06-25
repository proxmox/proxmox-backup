use anyhow::{bail, Error};

use crate::server::WorkerTask;

use super::{
    DataStore, BackupGroup, BackupDir, BackupInfo, IndexFile,
    ENCR_COMPR_BLOB_MAGIC_1_0, ENCRYPTED_BLOB_MAGIC_1_0,
    FileInfo, ArchiveType, archive_type,
};

fn verify_blob(datastore: &DataStore, backup_dir: &BackupDir, info: &FileInfo) -> Result<(), Error> {

    let (blob, raw_size) = datastore.load_blob(backup_dir, &info.filename)?;

    let csum = openssl::sha::sha256(blob.raw_data());
    if raw_size != info.size {
        bail!("wrong size ({} != {})", info.size, raw_size);
    }

    if csum != info.csum {
        bail!("wrong index checksum");
    }

    blob.verify_crc()?;

    let magic = blob.magic();

    if magic == &ENCR_COMPR_BLOB_MAGIC_1_0 || magic == &ENCRYPTED_BLOB_MAGIC_1_0 {
        return Ok(());
    }

    blob.decode(None)?;

    Ok(())
}

fn verify_fixed_index(datastore: &DataStore, backup_dir: &BackupDir, info: &FileInfo, worker: &WorkerTask) -> Result<(), Error> {

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

    for pos in 0..index.index_count() {

        worker.fail_on_abort()?;
        crate::tools::fail_on_shutdown()?;

        let (start, end, digest) = index.chunk_info(pos).unwrap();
        let size = end - start;
        datastore.verify_stored_chunk(&digest, size)?;
    }

    Ok(())
}

fn verify_dynamic_index(datastore: &DataStore, backup_dir: &BackupDir, info: &FileInfo, worker: &WorkerTask) -> Result<(), Error> {
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

    for pos in 0..index.index_count() {

        worker.fail_on_abort()?;
        crate::tools::fail_on_shutdown()?;

        let chunk_info = index.chunk_info(pos).unwrap();
        let size = chunk_info.range.end - chunk_info.range.start;
        datastore.verify_stored_chunk(&chunk_info.digest, size)?;
    }

    Ok(())
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
pub fn verify_backup_dir(datastore: &DataStore, backup_dir: &BackupDir, worker: &WorkerTask) -> Result<bool, Error> {

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
                ArchiveType::FixedIndex => verify_fixed_index(&datastore, &backup_dir, info, worker),
                ArchiveType::DynamicIndex => verify_dynamic_index(&datastore, &backup_dir, info, worker),
                ArchiveType::Blob => verify_blob(&datastore, &backup_dir, info),
            }
        });

        worker.fail_on_abort()?;
        crate::tools::fail_on_shutdown()?;

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
/// - Ok(true) if verify is successful
/// - Ok(false) if there were verification errors
/// - Err(_) if task was aborted
pub fn verify_backup_group(datastore: &DataStore, group: &BackupGroup, worker: &WorkerTask) -> Result<bool, Error> {

    let mut list = match group.list_backups(&datastore.base_path()) {
        Ok(list) => list,
        Err(err) => {
            worker.log(format!("verify group {}:{} - unable to list backups: {}", datastore.name(), group, err));
            return Ok(false);
        }
    };

    worker.log(format!("verify group {}:{}", datastore.name(), group));

    let mut error_count = 0;

    BackupInfo::sort_list(&mut list, false); // newest first
    for info in list {
        if !verify_backup_dir(datastore, &info.backup_dir, worker)? {
            error_count += 1;
        }
    }

    Ok(error_count == 0)
}

/// Verify all backups inside a datastore
///
/// Errors are logged to the worker log.
///
/// Returns
/// - Ok(true) if verify is successful
/// - Ok(false) if there were verification errors
/// - Err(_) if task was aborted
pub fn verify_all_backups(datastore: &DataStore, worker: &WorkerTask) -> Result<bool, Error> {

    let list = match BackupGroup::list_groups(&datastore.base_path()) {
        Ok(list) => list,
        Err(err) => {
            worker.log(format!("verify datastore {} - unable to list backups: {}", datastore.name(), err));
            return Ok(false);
        }
    };

    worker.log(format!("verify datastore {}", datastore.name()));

    let mut error_count = 0;
    for group in list {
        if !verify_backup_group(datastore, &group, worker)? {
            error_count += 1;
        }
    }

    Ok(error_count == 0)
}
