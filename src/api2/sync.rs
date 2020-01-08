use failure::*;
use serde_json::json;
use std::convert::TryFrom;
use std::sync::Arc;
use std::collections::HashMap;
use std::io::{Seek, SeekFrom};
use chrono::{Utc, TimeZone};

use proxmox::api::api;
use proxmox::api::{ApiMethod, Router, RpcEnvironment};

use crate::server::{WorkerTask};
use crate::backup::*;
use crate::client::*;
use crate::api2::types::*;

// fixme: implement filters
// Todo: correctly lock backup groups

async fn sync_index_chunks<I: IndexFile>(
    _worker: &WorkerTask,
    chunk_reader: &mut RemoteChunkReader,
    target: Arc<DataStore>,
    index: I,
) -> Result<(), Error> {


    for pos in 0..index.index_count() {
        let digest = index.index_digest(pos).unwrap();
        let chunk_exists = target.cond_touch_chunk(digest, false)?;
        if chunk_exists {
            //worker.log(format!("chunk {} exists {}", pos, proxmox::tools::digest_to_hex(digest)));
            continue;
        }
        //worker.log(format!("sync {} chunk {}", pos, proxmox::tools::digest_to_hex(digest)));
        let chunk = chunk_reader.read_raw_chunk(&digest)?;

        target.insert_chunk(&chunk, &digest)?;
    }

    Ok(())
}

async fn download_manifest(
    reader: &BackupReader,
    filename: &std::path::Path,
) -> Result<std::fs::File, Error> {

    let tmp_manifest_file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .read(true)
        .open(&filename)?;

    let mut tmp_manifest_file = reader.download(MANIFEST_BLOB_NAME, tmp_manifest_file).await?;

    tmp_manifest_file.seek(SeekFrom::Start(0))?;

    Ok(tmp_manifest_file)
}

async fn sync_single_archive(
    worker: &WorkerTask,
    reader: &BackupReader,
    chunk_reader: &mut RemoteChunkReader,
    tgt_store: Arc<DataStore>,
    snapshot: &BackupDir,
    archive_name: &str,
) -> Result<(), Error> {

    let mut path = tgt_store.base_path();
    path.push(snapshot.relative_path());
    path.push(archive_name);

    let mut tmp_path = path.clone();
    tmp_path.set_extension("tmp");

    worker.log(format!("sync archive {}", archive_name));
    let tmpfile = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .read(true)
        .open(&tmp_path)?;

    let tmpfile = reader.download(archive_name, tmpfile).await?;

    match archive_type(archive_name)? {
        ArchiveType::DynamicIndex => {
            let index = DynamicIndexReader::new(tmpfile)
                .map_err(|err| format_err!("unable to read dynamic index {:?} - {}", tmp_path, err))?;

            sync_index_chunks(worker, chunk_reader, tgt_store.clone(), index).await?;
        }
        ArchiveType::FixedIndex => {
            let index = FixedIndexReader::new(tmpfile)
                .map_err(|err| format_err!("unable to read fixed index '{:?}' - {}", tmp_path, err))?;

            sync_index_chunks(worker, chunk_reader, tgt_store.clone(), index).await?;
        }
        ArchiveType::Blob => { /* nothing to do */ }
    }
    if let Err(err) = std::fs::rename(&tmp_path, &path) {
        bail!("Atomic rename file {:?} failed - {}", path, err);
    }
    Ok(())
}

async fn sync_snapshot(
    worker: &WorkerTask,
    reader: Arc<BackupReader>,
    tgt_store: Arc<DataStore>,
    snapshot: &BackupDir,
) -> Result<(), Error> {

    let mut manifest_name = tgt_store.base_path();
    manifest_name.push(snapshot.relative_path());
    manifest_name.push(MANIFEST_BLOB_NAME);

    let mut tmp_manifest_name = manifest_name.clone();
    tmp_manifest_name.set_extension("tmp");

    let mut tmp_manifest_file = download_manifest(&reader, &tmp_manifest_name).await?;
    let tmp_manifest_blob = DataBlob::load(&mut tmp_manifest_file)?;
    tmp_manifest_blob.verify_crc()?;

    if manifest_name.exists() {
        let manifest_blob = proxmox::tools::try_block!({
            let mut manifest_file = std::fs::File::open(&manifest_name)
                .map_err(|err| format_err!("unable to open local manifest {:?} - {}", manifest_name, err))?;

            let manifest_blob = DataBlob::load(&mut manifest_file)?;
            manifest_blob.verify_crc()?;
            Ok(manifest_blob)
        }).map_err(|err: Error| {
            format_err!("unable to read local manifest {:?} - {}", manifest_name, err)
        })?;

        if manifest_blob.raw_data() == tmp_manifest_blob.raw_data() {
            return Ok(()); // nothing changed
        }
    }

    let manifest = BackupManifest::try_from(tmp_manifest_blob)?;

    let mut chunk_reader = RemoteChunkReader::new(reader.clone(), None, HashMap::new());

    for item in manifest.files() {
        let mut path = tgt_store.base_path();
        path.push(snapshot.relative_path());
        path.push(&item.filename);

        if path.exists() {
            match archive_type(&item.filename)? {
                ArchiveType::DynamicIndex => {
                    let index = DynamicIndexReader::open(&path)?;
                    let (csum, size) = index.compute_csum();
                    match manifest.verify_file(&item.filename, &csum, size) {
                        Ok(_) => continue,
                        Err(err) => {
                            worker.log(format!("detected changed file {:?} - {}", path, err));
                        }
                    }
                }
                ArchiveType::FixedIndex => {
                    let index = FixedIndexReader::open(&path)?;
                    let (csum, size) = index.compute_csum();
                    match manifest.verify_file(&item.filename, &csum, size) {
                        Ok(_) => continue,
                        Err(err) => {
                            worker.log(format!("detected changed file {:?} - {}", path, err));
                        }
                    }
                }
                ArchiveType::Blob => {
                    let mut tmpfile = std::fs::File::open(&path)?;
                    let (csum, size) = compute_file_csum(&mut tmpfile)?;
                    match manifest.verify_file(&item.filename, &csum, size) {
                        Ok(_) => continue,
                        Err(err) => {
                            worker.log(format!("detected changed file {:?} - {}", path, err));
                        }
                    }
                }
            }
        }

        sync_single_archive(
            worker,
            &reader,
            &mut chunk_reader,
            tgt_store.clone(),
            snapshot,
            &item.filename,
        ).await?;
    }

    if let Err(err) = std::fs::rename(&tmp_manifest_name, &manifest_name) {
        bail!("Atomic rename file {:?} failed - {}", manifest_name, err);
    }

    // cleanup - remove stale files
    tgt_store.cleanup_backup_dir(snapshot, &manifest)?;

    Ok(())
}

pub async fn sync_snapshot_from(
    worker: &WorkerTask,
    reader: Arc<BackupReader>,
    tgt_store: Arc<DataStore>,
    snapshot: &BackupDir,
) -> Result<(), Error> {

    let (_path, is_new) = tgt_store.create_backup_dir(&snapshot)?;

    if is_new {
        worker.log(format!("sync snapshot {:?}", snapshot.relative_path()));

        if let Err(err) = sync_snapshot(worker, reader, tgt_store.clone(), &snapshot).await {
            if let Err(cleanup_err) = tgt_store.remove_backup_dir(&snapshot) {
                worker.log(format!("cleanup error - {}", cleanup_err));
            }
            return Err(err);
        }
    } else {
        worker.log(format!("re-sync snapshot {:?}", snapshot.relative_path()));
        sync_snapshot(worker, reader, tgt_store.clone(), &snapshot).await?
    }

    Ok(())
}

pub async fn sync_group(
    worker: &WorkerTask,
    client: &HttpClient,
    src_repo: &BackupRepository,
    tgt_store: Arc<DataStore>,
    group: &BackupGroup,
) -> Result<(), Error> {

    let path = format!("api2/json/admin/datastore/{}/snapshots", src_repo.store());

    let args = json!({
        "backup-type": group.backup_type(),
        "backup-id": group.backup_id(),
    });

    let mut result = client.get(&path, Some(args)).await?;
    let mut list: Vec<SnapshotListItem> = serde_json::from_value(result["data"].take())?;

    list.sort_unstable_by(|a, b| a.backup_time.cmp(&b.backup_time));

    let auth_info = client.login().await?;

    let last_sync = group.last_successful_backup(&tgt_store.base_path())?;

    for item in list {
        let backup_time = Utc.timestamp(item.backup_time, 0);
        if let Some(last_sync_time) = last_sync {
            if last_sync_time > backup_time { continue; }
        }

        let new_client = HttpClient::new(
            src_repo.host(),
            src_repo.user(),
            Some(auth_info.ticket.clone())
        )?;

        let reader = BackupReader::start(
            new_client,
            None,
            src_repo.store(),
            &item.backup_type,
            &item.backup_id,
            backup_time,
            true,
        ).await?;

        let snapshot = BackupDir::new(item.backup_type, item.backup_id, item.backup_time);

        sync_snapshot_from(worker, reader, tgt_store.clone(), &snapshot).await?;
    }

    Ok(())
}

pub async fn sync_store(
    worker: &WorkerTask,
    client: &HttpClient,
    src_repo: &BackupRepository,
    tgt_store: Arc<DataStore>,
) -> Result<(), Error> {

    let path = format!("api2/json/admin/datastore/{}/groups", src_repo.store());

    let mut result = client.get(&path, None).await?;

    let list = result["data"].as_array_mut().unwrap();

    list.sort_unstable_by(|a, b| {
        let a_id = a["backup-id"].as_str().unwrap();
        let a_backup_type = a["backup-type"].as_str().unwrap();
        let b_id = b["backup-id"].as_str().unwrap();
        let b_backup_type = b["backup-type"].as_str().unwrap();

        let type_order = a_backup_type.cmp(b_backup_type);
        if type_order == std::cmp::Ordering::Equal {
            a_id.cmp(b_id)
        } else {
            type_order
        }
    });

    let mut errors = false;

    for item in list {

        let id = item["backup-id"].as_str().unwrap();
        let btype = item["backup-type"].as_str().unwrap();

        let group = BackupGroup::new(btype, id);
        if let Err(err) = sync_group(worker, client, src_repo, tgt_store.clone(), &group).await {
            worker.log(format!("sync group {}/{} failed - {}", btype, id, err));
            errors = true;
            // continue
        }
    }

    if errors {
        bail!("sync failed with some errors.");
    }

    Ok(())
}

#[api(
    input: {
        properties: {
            store: {
                schema: DATASTORE_SCHEMA,
            },
            "remote-host": {
                description: "Remote host", // TODO: use predefined type: host or IP
                type: String,
            },
            "remote-store": {
                schema: DATASTORE_SCHEMA,
            },
            "remote-user": {
                description: "Remote user name.", // TODO: use predefined typed
                type: String,
            },
            "remote-password": {
                description: "Remote passsword.",
                type: String,
            },
        },
    },
)]
/// Sync store from otherrepository
async fn sync_from (
    store: String,
    remote_host: String,
    remote_store: String,
    remote_user: String,
    remote_password: String,
    _info: &ApiMethod,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<String, Error> {

    let username = rpcenv.get_user().unwrap();

    let tgt_store = DataStore::lookup_datastore(&store)?;

    let client = HttpClient::new(&remote_host, &remote_user, Some(remote_password))?;
    let _auth_info = client.login() // make sure we can auth
        .await
        .map_err(|err| format_err!("remote connection to '{}' failed - {}", remote_host, err))?;

    let src_repo = BackupRepository::new(Some(remote_user), Some(remote_host), remote_store);

    // fixme: set to_stdout to false?
    let upid_str = WorkerTask::spawn("sync", Some(store.clone()), &username.clone(), true, move |worker| async move {

        worker.log(format!("sync datastore '{}' start", store));

        // explicit create shared lock to prevent GC on newly created chunks
        let _shared_store_lock = tgt_store.try_shared_chunk_store_lock()?;

        sync_store(&worker, &client, &src_repo, tgt_store.clone()).await?;

        worker.log(format!("sync datastore '{}' end", store));

        Ok(())
    })?;

    Ok(upid_str)
}

pub const ROUTER: Router = Router::new()
    .post(&API_METHOD_SYNC_FROM);
