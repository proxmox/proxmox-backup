//! Sync datastore from remote server

use std::cmp::min;
use std::collections::{HashMap, HashSet};
use std::convert::TryFrom;
use std::io::{Seek, SeekFrom};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use anyhow::{bail, format_err, Error};
use http::StatusCode;
use pbs_config::CachedUserInfo;
use serde_json::json;

use proxmox_router::HttpError;
use proxmox_sys::task_log;

use pbs_api_types::{
    Authid, BackupNamespace, DatastoreWithNamespace, GroupFilter, GroupListItem, NamespaceListItem,
    Operation, RateLimitConfig, Remote, SnapshotListItem, MAX_NAMESPACE_DEPTH,
    PRIV_DATASTORE_AUDIT, PRIV_DATASTORE_BACKUP, PRIV_DATASTORE_MODIFY,
};

use pbs_client::{
    BackupReader, BackupRepository, HttpClient, HttpClientOptions, RemoteChunkReader,
};
use pbs_datastore::data_blob::DataBlob;
use pbs_datastore::dynamic_index::DynamicIndexReader;
use pbs_datastore::fixed_index::FixedIndexReader;
use pbs_datastore::index::IndexFile;
use pbs_datastore::manifest::{
    archive_type, ArchiveType, BackupManifest, FileInfo, CLIENT_LOG_BLOB_NAME, MANIFEST_BLOB_NAME,
};
use pbs_datastore::{check_backup_owner, DataStore, StoreProgress};
use pbs_tools::sha::sha256;
use proxmox_rest_server::WorkerTask;

use crate::tools::parallel_handler::ParallelHandler;

/// Parameters for a pull operation.
pub struct PullParameters {
    /// Remote that is pulled from
    remote: Remote,
    /// Full specification of remote datastore
    source: BackupRepository,
    /// Local store that is pulled into
    store: Arc<DataStore>,
    /// Remote namespace
    remote_ns: BackupNamespace,
    /// Local namespace (anchor)
    ns: BackupNamespace,
    /// Owner of synced groups (needs to match local owner of pre-existing groups)
    owner: Authid,
    /// Whether to remove groups which exist locally, but not on the remote end
    remove_vanished: bool,
    /// How many levels of sub-namespaces to pull (0 == no recursion)
    max_depth: usize,
    /// Filters for reducing the pull scope
    group_filter: Option<Vec<GroupFilter>>,
    /// Rate limits for all transfers from `remote`
    limit: RateLimitConfig,
}

impl PullParameters {
    /// Creates a new instance of `PullParameters`.
    ///
    /// `remote` will be dereferenced via [pbs_api_types::RemoteConfig], and combined into a
    /// [BackupRepository] with `remote_store`.
    pub fn new(
        store: &str,
        ns: BackupNamespace,
        remote: &str,
        remote_store: &str,
        remote_ns: BackupNamespace,
        owner: Authid,
        remove_vanished: Option<bool>,
        max_depth: usize,
        group_filter: Option<Vec<GroupFilter>>,
        limit: RateLimitConfig,
    ) -> Result<Self, Error> {
        let store = DataStore::lookup_datastore(store, Some(Operation::Write))?;

        let max_depth = min(max_depth, MAX_NAMESPACE_DEPTH - remote_ns.depth());

        let (remote_config, _digest) = pbs_config::remote::config()?;
        let remote: Remote = remote_config.lookup("remote", remote)?;

        let remove_vanished = remove_vanished.unwrap_or(false);

        let source = BackupRepository::new(
            Some(remote.config.auth_id.clone()),
            Some(remote.config.host.clone()),
            remote.config.port,
            remote_store.to_string(),
        );

        Ok(Self {
            remote,
            remote_ns,
            ns,
            source,
            store,
            owner,
            remove_vanished,
            max_depth,
            group_filter,
            limit,
        })
    }

    /// Creates a new [HttpClient] for accessing the [Remote] that is pulled from.
    pub async fn client(&self) -> Result<HttpClient, Error> {
        crate::api2::config::remote::remote_client(&self.remote, Some(self.limit.clone())).await
    }

    /// Returns DatastoreWithNamespace with namespace (or local namespace anchor).
    pub fn store_with_ns(&self, ns: BackupNamespace) -> DatastoreWithNamespace {
        DatastoreWithNamespace {
            store: self.store.name().to_string(),
            ns,
        }
    }
}

async fn pull_index_chunks<I: IndexFile>(
    worker: &WorkerTask,
    chunk_reader: RemoteChunkReader,
    target: Arc<DataStore>,
    index: I,
    downloaded_chunks: Arc<Mutex<HashSet<[u8; 32]>>>,
) -> Result<(), Error> {
    use futures::stream::{self, StreamExt, TryStreamExt};

    let start_time = SystemTime::now();

    let stream = stream::iter(
        (0..index.index_count())
            .map(|pos| index.chunk_info(pos).unwrap())
            .filter(|info| {
                let mut guard = downloaded_chunks.lock().unwrap();
                let done = guard.contains(&info.digest);
                if !done {
                    // Note: We mark a chunk as downloaded before its actually downloaded
                    // to avoid duplicate downloads.
                    guard.insert(info.digest);
                }
                !done
            }),
    );

    let target2 = target.clone();
    let verify_pool = ParallelHandler::new(
        "sync chunk writer",
        4,
        move |(chunk, digest, size): (DataBlob, [u8; 32], u64)| {
            // println!("verify and write {}", hex::encode(&digest));
            chunk.verify_unencrypted(size as usize, &digest)?;
            target2.insert_chunk(&chunk, &digest)?;
            Ok(())
        },
    );

    let verify_and_write_channel = verify_pool.channel();

    let bytes = Arc::new(AtomicUsize::new(0));

    stream
        .map(|info| {
            let target = Arc::clone(&target);
            let chunk_reader = chunk_reader.clone();
            let bytes = Arc::clone(&bytes);
            let verify_and_write_channel = verify_and_write_channel.clone();

            Ok::<_, Error>(async move {
                let chunk_exists = proxmox_async::runtime::block_in_place(|| {
                    target.cond_touch_chunk(&info.digest, false)
                })?;
                if chunk_exists {
                    //task_log!(worker, "chunk {} exists {}", pos, hex::encode(digest));
                    return Ok::<_, Error>(());
                }
                //task_log!(worker, "sync {} chunk {}", pos, hex::encode(digest));
                let chunk = chunk_reader.read_raw_chunk(&info.digest).await?;
                let raw_size = chunk.raw_size() as usize;

                // decode, verify and write in a separate threads to maximize throughput
                proxmox_async::runtime::block_in_place(|| {
                    verify_and_write_channel.send((chunk, info.digest, info.size()))
                })?;

                bytes.fetch_add(raw_size, Ordering::SeqCst);

                Ok(())
            })
        })
        .try_buffer_unordered(20)
        .try_for_each(|_res| futures::future::ok(()))
        .await?;

    drop(verify_and_write_channel);

    verify_pool.complete()?;

    let elapsed = start_time.elapsed()?.as_secs_f64();

    let bytes = bytes.load(Ordering::SeqCst);

    task_log!(
        worker,
        "downloaded {} bytes ({:.2} MiB/s)",
        bytes,
        (bytes as f64) / (1024.0 * 1024.0 * elapsed)
    );

    Ok(())
}

async fn download_manifest(
    reader: &BackupReader,
    filename: &std::path::Path,
) -> Result<std::fs::File, Error> {
    let mut tmp_manifest_file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .read(true)
        .open(&filename)?;

    reader
        .download(MANIFEST_BLOB_NAME, &mut tmp_manifest_file)
        .await?;

    tmp_manifest_file.seek(SeekFrom::Start(0))?;

    Ok(tmp_manifest_file)
}

fn verify_archive(info: &FileInfo, csum: &[u8; 32], size: u64) -> Result<(), Error> {
    if size != info.size {
        bail!(
            "wrong size for file '{}' ({} != {})",
            info.filename,
            info.size,
            size
        );
    }

    if csum != &info.csum {
        bail!("wrong checksum for file '{}'", info.filename);
    }

    Ok(())
}

/// Pulls a single file referenced by a manifest.
///
/// Pulling an archive consists of the following steps:
/// - Create tmp file for archive
/// - Download archive file into tmp file
/// - Verify tmp file checksum
/// - if archive is an index, pull referenced chunks
/// - Rename tmp file into real path
async fn pull_single_archive(
    worker: &WorkerTask,
    reader: &BackupReader,
    chunk_reader: &mut RemoteChunkReader,
    snapshot: &pbs_datastore::BackupDir,
    archive_info: &FileInfo,
    downloaded_chunks: Arc<Mutex<HashSet<[u8; 32]>>>,
) -> Result<(), Error> {
    let archive_name = &archive_info.filename;
    let mut path = snapshot.full_path();
    path.push(archive_name);

    let mut tmp_path = path.clone();
    tmp_path.set_extension("tmp");

    task_log!(worker, "sync archive {}", archive_name);

    let mut tmpfile = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .read(true)
        .open(&tmp_path)?;

    reader.download(archive_name, &mut tmpfile).await?;

    match archive_type(archive_name)? {
        ArchiveType::DynamicIndex => {
            let index = DynamicIndexReader::new(tmpfile).map_err(|err| {
                format_err!("unable to read dynamic index {:?} - {}", tmp_path, err)
            })?;
            let (csum, size) = index.compute_csum();
            verify_archive(archive_info, &csum, size)?;

            pull_index_chunks(
                worker,
                chunk_reader.clone(),
                snapshot.datastore().clone(),
                index,
                downloaded_chunks,
            )
            .await?;
        }
        ArchiveType::FixedIndex => {
            let index = FixedIndexReader::new(tmpfile).map_err(|err| {
                format_err!("unable to read fixed index '{:?}' - {}", tmp_path, err)
            })?;
            let (csum, size) = index.compute_csum();
            verify_archive(archive_info, &csum, size)?;

            pull_index_chunks(
                worker,
                chunk_reader.clone(),
                snapshot.datastore().clone(),
                index,
                downloaded_chunks,
            )
            .await?;
        }
        ArchiveType::Blob => {
            tmpfile.seek(SeekFrom::Start(0))?;
            let (csum, size) = sha256(&mut tmpfile)?;
            verify_archive(archive_info, &csum, size)?;
        }
    }
    if let Err(err) = std::fs::rename(&tmp_path, &path) {
        bail!("Atomic rename file {:?} failed - {}", path, err);
    }
    Ok(())
}

// Note: The client.log.blob is uploaded after the backup, so it is
// not mentioned in the manifest.
async fn try_client_log_download(
    worker: &WorkerTask,
    reader: Arc<BackupReader>,
    path: &std::path::Path,
) -> Result<(), Error> {
    let mut tmp_path = path.to_owned();
    tmp_path.set_extension("tmp");

    let tmpfile = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .read(true)
        .open(&tmp_path)?;

    // Note: be silent if there is no log - only log successful download
    if let Ok(()) = reader.download(CLIENT_LOG_BLOB_NAME, tmpfile).await {
        if let Err(err) = std::fs::rename(&tmp_path, &path) {
            bail!("Atomic rename file {:?} failed - {}", path, err);
        }
        task_log!(worker, "got backup log file {:?}", CLIENT_LOG_BLOB_NAME);
    }

    Ok(())
}

/// Actual implementation of pulling a snapshot.
///
/// Pulling a snapshot consists of the following steps:
/// - (Re)download the manifest
/// -- if it matches, only download log and treat snapshot as already synced
/// - Iterate over referenced files
/// -- if file already exists, verify contents
/// -- if not, pull it from the remote
/// - Download log if not already existing
async fn pull_snapshot(
    worker: &WorkerTask,
    reader: Arc<BackupReader>,
    snapshot: &pbs_datastore::BackupDir,
    downloaded_chunks: Arc<Mutex<HashSet<[u8; 32]>>>,
) -> Result<(), Error> {
    let mut manifest_name = snapshot.full_path();
    manifest_name.push(MANIFEST_BLOB_NAME);

    let mut client_log_name = snapshot.full_path();
    client_log_name.push(CLIENT_LOG_BLOB_NAME);

    let mut tmp_manifest_name = manifest_name.clone();
    tmp_manifest_name.set_extension("tmp");

    let download_res = download_manifest(&reader, &tmp_manifest_name).await;
    let mut tmp_manifest_file = match download_res {
        Ok(manifest_file) => manifest_file,
        Err(err) => {
            match err.downcast_ref::<HttpError>() {
                Some(HttpError { code, message }) => match *code {
                    StatusCode::NOT_FOUND => {
                        task_log!(
                            worker,
                            "skipping snapshot {snapshot} - vanished since start of sync",
                        );
                        return Ok(());
                    }
                    _ => {
                        bail!("HTTP error {code} - {message}");
                    }
                },
                None => {
                    return Err(err);
                }
            };
        }
    };
    let tmp_manifest_blob = DataBlob::load_from_reader(&mut tmp_manifest_file)?;

    if manifest_name.exists() {
        let manifest_blob = proxmox_lang::try_block!({
            let mut manifest_file = std::fs::File::open(&manifest_name).map_err(|err| {
                format_err!("unable to open local manifest {manifest_name:?} - {err}")
            })?;

            let manifest_blob = DataBlob::load_from_reader(&mut manifest_file)?;
            Ok(manifest_blob)
        })
        .map_err(|err: Error| {
            format_err!("unable to read local manifest {manifest_name:?} - {err}")
        })?;

        if manifest_blob.raw_data() == tmp_manifest_blob.raw_data() {
            if !client_log_name.exists() {
                try_client_log_download(worker, reader, &client_log_name).await?;
            }
            task_log!(worker, "no data changes");
            let _ = std::fs::remove_file(&tmp_manifest_name);
            return Ok(()); // nothing changed
        }
    }

    let manifest = BackupManifest::try_from(tmp_manifest_blob)?;

    for item in manifest.files() {
        let mut path = snapshot.full_path();
        path.push(&item.filename);

        if path.exists() {
            match archive_type(&item.filename)? {
                ArchiveType::DynamicIndex => {
                    let index = DynamicIndexReader::open(&path)?;
                    let (csum, size) = index.compute_csum();
                    match manifest.verify_file(&item.filename, &csum, size) {
                        Ok(_) => continue,
                        Err(err) => {
                            task_log!(worker, "detected changed file {:?} - {}", path, err);
                        }
                    }
                }
                ArchiveType::FixedIndex => {
                    let index = FixedIndexReader::open(&path)?;
                    let (csum, size) = index.compute_csum();
                    match manifest.verify_file(&item.filename, &csum, size) {
                        Ok(_) => continue,
                        Err(err) => {
                            task_log!(worker, "detected changed file {:?} - {}", path, err);
                        }
                    }
                }
                ArchiveType::Blob => {
                    let mut tmpfile = std::fs::File::open(&path)?;
                    let (csum, size) = sha256(&mut tmpfile)?;
                    match manifest.verify_file(&item.filename, &csum, size) {
                        Ok(_) => continue,
                        Err(err) => {
                            task_log!(worker, "detected changed file {:?} - {}", path, err);
                        }
                    }
                }
            }
        }

        let mut chunk_reader = RemoteChunkReader::new(
            reader.clone(),
            None,
            item.chunk_crypt_mode(),
            HashMap::new(),
        );

        pull_single_archive(
            worker,
            &reader,
            &mut chunk_reader,
            snapshot,
            item,
            downloaded_chunks.clone(),
        )
        .await?;
    }

    if let Err(err) = std::fs::rename(&tmp_manifest_name, &manifest_name) {
        bail!("Atomic rename file {:?} failed - {}", manifest_name, err);
    }

    if !client_log_name.exists() {
        try_client_log_download(worker, reader, &client_log_name).await?;
    }

    snapshot.cleanup_unreferenced_files(&manifest)?;

    Ok(())
}

/// Pulls a `snapshot`, removing newly created ones on error, but keeping existing ones in any case.
///
/// The `reader` is configured to read from the remote / source namespace, while the `snapshot` is
/// pointing to the local datastore and target namespace.
async fn pull_snapshot_from(
    worker: &WorkerTask,
    reader: Arc<BackupReader>,
    snapshot: &pbs_datastore::BackupDir,
    downloaded_chunks: Arc<Mutex<HashSet<[u8; 32]>>>,
) -> Result<(), Error> {
    let (_path, is_new, _snap_lock) = snapshot
        .datastore()
        .create_locked_backup_dir(snapshot.backup_ns(), snapshot.as_ref())?;

    let snapshot_path = snapshot.to_string();
    if is_new {
        task_log!(worker, "sync snapshot {:?}", snapshot_path);

        if let Err(err) = pull_snapshot(worker, reader, snapshot, downloaded_chunks).await {
            if let Err(cleanup_err) = snapshot.datastore().remove_backup_dir(
                snapshot.backup_ns(),
                snapshot.as_ref(),
                true,
            ) {
                task_log!(worker, "cleanup error - {}", cleanup_err);
            }
            return Err(err);
        }
        task_log!(worker, "sync snapshot {:?} done", snapshot_path);
    } else {
        task_log!(worker, "re-sync snapshot {:?}", snapshot_path);
        pull_snapshot(worker, reader, snapshot, downloaded_chunks).await?;
        task_log!(worker, "re-sync snapshot {:?} done", snapshot_path);
    }

    Ok(())
}

struct SkipInfo {
    oldest: i64,
    newest: i64,
    count: u64,
}

impl SkipInfo {
    fn update(&mut self, backup_time: i64) {
        self.count += 1;

        if backup_time < self.oldest {
            self.oldest = backup_time;
        }

        if backup_time > self.newest {
            self.newest = backup_time;
        }
    }

    fn affected(&self) -> Result<String, Error> {
        match self.count {
            0 => Ok(String::new()),
            1 => Ok(proxmox_time::epoch_to_rfc3339_utc(self.oldest)?),
            _ => Ok(format!(
                "{} .. {}",
                proxmox_time::epoch_to_rfc3339_utc(self.oldest)?,
                proxmox_time::epoch_to_rfc3339_utc(self.newest)?,
            )),
        }
    }
}

impl std::fmt::Display for SkipInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "skipped: {} snapshot(s) ({}) older than the newest local snapshot",
            self.count,
            self.affected().map_err(|_| std::fmt::Error)?
        )
    }
}

/// Pulls a group according to `params`.
///
/// Pulling a group consists of the following steps:
/// - Query the list of snapshots available for this group in the source namespace on the remote
/// - Sort by snapshot time
/// - Get last snapshot timestamp on local datastore
/// - Iterate over list of snapshots
/// -- Recreate client/BackupReader
/// -- pull snapshot, unless it's not finished yet or older than last local snapshot
/// - (remove_vanished) list all local snapshots, remove those that don't exist on remote
///
/// Backwards-compat: if `source_ns` is [None], only the group type and ID will be sent to the
/// remote when querying snapshots. This allows us to interact with old remotes that don't have
/// namespace support yet.
///
/// Permission checks:
/// - remote snapshot access is checked by remote (twice: query and opening the backup reader)
/// - local group owner is already checked by pull_store
async fn pull_group(
    worker: &WorkerTask,
    client: &HttpClient,
    params: &PullParameters,
    group: &pbs_api_types::BackupGroup,
    remote_ns: BackupNamespace,
    progress: &mut StoreProgress,
) -> Result<(), Error> {
    let path = format!(
        "api2/json/admin/datastore/{}/snapshots",
        params.source.store()
    );

    let mut args = json!({
        "backup-type": group.ty,
        "backup-id": group.id,
    });

    if !remote_ns.is_root() {
        args["backup-ns"] = serde_json::to_value(&remote_ns)?;
    }

    let target_ns = remote_ns.map_prefix(&params.remote_ns, &params.ns)?;

    let mut result = client.get(&path, Some(args)).await?;
    let mut list: Vec<SnapshotListItem> = serde_json::from_value(result["data"].take())?;

    list.sort_unstable_by(|a, b| a.backup.time.cmp(&b.backup.time));

    client.login().await?; // make sure auth is complete

    let fingerprint = client.fingerprint();

    let last_sync = params.store.last_successful_backup(&target_ns, group)?;

    let mut remote_snapshots = std::collections::HashSet::new();

    // start with 65536 chunks (up to 256 GiB)
    let downloaded_chunks = Arc::new(Mutex::new(HashSet::with_capacity(1024 * 64)));

    progress.group_snapshots = list.len() as u64;

    let mut skip_info = SkipInfo {
        oldest: i64::MAX,
        newest: i64::MIN,
        count: 0,
    };

    for (pos, item) in list.into_iter().enumerate() {
        let snapshot = item.backup;

        // in-progress backups can't be synced
        if item.size.is_none() {
            task_log!(
                worker,
                "skipping snapshot {} - in-progress backup",
                snapshot
            );
            continue;
        }

        remote_snapshots.insert(snapshot.time);

        if let Some(last_sync_time) = last_sync {
            if last_sync_time > snapshot.time {
                skip_info.update(snapshot.time);
                continue;
            }
        }

        // get updated auth_info (new tickets)
        let auth_info = client.login().await?;

        let options =
            HttpClientOptions::new_non_interactive(auth_info.ticket.clone(), fingerprint.clone())
                .rate_limit(params.limit.clone());

        let new_client = HttpClient::new(
            params.source.host(),
            params.source.port(),
            params.source.auth_id(),
            options,
        )?;

        let reader = BackupReader::start(
            new_client,
            None,
            params.source.store(),
            &remote_ns,
            &snapshot,
            true,
        )
        .await?;

        let snapshot = params.store.backup_dir(target_ns.clone(), snapshot)?;

        let result = pull_snapshot_from(worker, reader, &snapshot, downloaded_chunks.clone()).await;

        progress.done_snapshots = pos as u64 + 1;
        task_log!(worker, "percentage done: {}", progress);

        result?; // stop on error
    }

    if params.remove_vanished {
        let group = params.store.backup_group(target_ns.clone(), group.clone());
        let local_list = group.list_backups()?;
        for info in local_list {
            let backup_time = info.backup_dir.backup_time();
            if remote_snapshots.contains(&backup_time) {
                continue;
            }
            if info.backup_dir.is_protected() {
                task_log!(
                    worker,
                    "don't delete vanished snapshot {:?} (protected)",
                    info.backup_dir.relative_path()
                );
                continue;
            }
            task_log!(
                worker,
                "delete vanished snapshot {:?}",
                info.backup_dir.relative_path()
            );
            params
                .store
                .remove_backup_dir(&target_ns, info.backup_dir.as_ref(), false)?;
        }
    }

    if skip_info.count > 0 {
        task_log!(worker, "{}", skip_info);
    }

    Ok(())
}

async fn query_namespaces(
    client: &HttpClient,
    params: &PullParameters,
) -> Result<Vec<BackupNamespace>, Error> {
    let path = format!(
        "api2/json/admin/datastore/{}/namespace",
        params.source.store()
    );
    let data = json!({
        "max-depth": params.max_depth,
    });
    let mut result = client
        .get(&path, Some(data))
        .await
        .map_err(|err| format_err!("Failed to retrieve namespaces from remote - {}", err))?;
    let mut list: Vec<NamespaceListItem> = serde_json::from_value(result["data"].take())?;

    // parents first
    list.sort_unstable_by(|a, b| a.ns.name_len().cmp(&b.ns.name_len()));

    Ok(list.iter().map(|item| item.ns.clone()).collect())
}

fn check_ns_privs(
    store_with_ns: &DatastoreWithNamespace,
    owner: &Authid,
    privs: u64,
) -> Result<(), Error> {
    let user_info = CachedUserInfo::new()?;

    // TODO re-sync with API, maybe find common place?

    let user_privs = user_info.lookup_privs(owner, &store_with_ns.acl_path());

    if (user_privs & privs) == 0 {
        bail!("no permission to modify parent/datastore.");
    }
    Ok(())
}

fn check_and_create_ns(
    params: &PullParameters,
    store_with_ns: &DatastoreWithNamespace,
) -> Result<bool, Error> {
    let ns = &store_with_ns.ns;
    let mut created = false;

    if !ns.is_root() && !params.store.namespace_path(&ns).exists() {
        let mut parent = ns.clone();
        let name = parent.pop();

        let parent = params.store_with_ns(parent);

        if let Err(err) = check_ns_privs(&parent, &params.owner, PRIV_DATASTORE_MODIFY) {
            bail!(
                "Not allowed to create namespace {} - {}",
                store_with_ns,
                err,
            );
        }
        if let Some(name) = name {
            if let Err(err) = params.store.create_namespace(&parent.ns, name) {
                bail!(
                    "sync namespace {} failed - namespace creation failed: {}",
                    &store_with_ns,
                    err
                );
            }
            created = true;
        } else {
            bail!(
                "sync namespace {} failed - namespace creation failed - couldn't determine parent namespace",
                &store_with_ns,
            );
        }
    }

    // TODO re-sync with API, maybe find common place?
    if let Err(err) = check_ns_privs(&store_with_ns, &params.owner, PRIV_DATASTORE_BACKUP) {
        bail!("sync namespace {} failed - {}", &store_with_ns, err);
    }

    Ok(created)
}

fn check_and_remove_ns(params: &PullParameters, local_ns: &BackupNamespace) -> Result<bool, Error> {
    let parent = local_ns.clone().parent();
    check_ns_privs(
        &params.store_with_ns(parent),
        &params.owner,
        PRIV_DATASTORE_MODIFY,
    )?;
    params.store.remove_namespace_recursive(local_ns)
}

fn check_and_remove_vanished_ns(
    worker: &WorkerTask,
    params: &PullParameters,
    synced_ns: HashSet<BackupNamespace>,
) -> Result<bool, Error> {
    let mut errors = false;
    let user_info = CachedUserInfo::new()?;

    let mut local_ns_list: Vec<BackupNamespace> = params
        .store
        .recursive_iter_backup_ns_ok(params.ns.clone(), Some(params.max_depth))?
        .filter(|ns| {
            let store_with_ns = params.store_with_ns(ns.clone());
            let user_privs = user_info.lookup_privs(&params.owner, &store_with_ns.acl_path());
            user_privs & (PRIV_DATASTORE_BACKUP | PRIV_DATASTORE_AUDIT) != 0
        })
        .collect();

    // children first!
    local_ns_list.sort_unstable_by_key(|b| std::cmp::Reverse(b.name_len()));

    for local_ns in local_ns_list {
        if local_ns == params.ns {
            continue;
        }

        if synced_ns.contains(&local_ns) {
            continue;
        }

        if local_ns.is_root() {
            continue;
        }
        match check_and_remove_ns(params, &local_ns) {
            Ok(true) => task_log!(worker, "Removed namespace {}", local_ns),
            Ok(false) => task_log!(
                worker,
                "Did not remove namespace {} - protected snapshots remain",
                local_ns
            ),
            Err(err) => {
                task_log!(worker, "Failed to remove namespace {} - {}", local_ns, err);
                errors = true;
            }
        }
    }

    Ok(errors)
}

/// Pulls a store according to `params`.
///
/// Pulling a store consists of the following steps:
/// - Query list of namespaces on the remote
/// - Iterate list
/// -- create sub-NS if needed (and allowed)
/// -- attempt to pull each NS in turn
/// - (remove_vanished && max_depth > 0) remove sub-NS which are not or no longer available on the remote
///
/// Backwards compat: if the remote namespace is `/` and recursion is disabled, no namespace is
/// passed to the remote at all to allow pulling from remotes which have no notion of namespaces.
///
/// Permission checks:
/// - access to local datastore, namespace anchor and remote entry need to be checked at call site
/// - remote namespaces are filtered by remote
/// - creation and removal of sub-NS checked here
/// - access to sub-NS checked here
pub async fn pull_store(
    worker: &WorkerTask,
    client: &HttpClient,
    params: &PullParameters,
) -> Result<(), Error> {
    // explicit create shared lock to prevent GC on newly created chunks
    let _shared_store_lock = params.store.try_shared_chunk_store_lock()?;

    let namespaces = if params.remote_ns.is_root() && params.max_depth == 0 {
        vec![params.remote_ns.clone()] // backwards compat - don't query remote namespaces!
    } else {
        query_namespaces(client, params).await?
    };

    let (mut groups, mut snapshots) = (0, 0);
    let mut synced_ns = HashSet::with_capacity(namespaces.len());
    let mut errors = false;

    for namespace in namespaces {
        let source_store_ns = DatastoreWithNamespace {
            store: params.source.store().to_owned(),
            ns: namespace.clone(),
        };
        let target_ns = namespace.map_prefix(&params.remote_ns, &params.ns)?;
        let target_store_ns = params.store_with_ns(target_ns.clone());

        task_log!(worker, "----");
        task_log!(
            worker,
            "Syncing {} into {}",
            source_store_ns,
            target_store_ns
        );

        synced_ns.insert(target_ns.clone());

        match check_and_create_ns(params, &target_store_ns) {
            Ok(true) => task_log!(worker, "Created namespace {}", target_ns),
            Ok(false) => {}
            Err(err) => {
                task_log!(
                    worker,
                    "Cannot sync {} into {} - {}",
                    source_store_ns,
                    target_store_ns,
                    err,
                );
                errors = true;
                continue;
            }
        }

        match pull_ns(worker, client, params, namespace.clone(), target_ns).await {
            Ok((ns_progress, ns_errors)) => {
                errors |= ns_errors;

                if params.max_depth > 0 {
                    groups += ns_progress.done_groups;
                    snapshots += ns_progress.done_snapshots;
                    task_log!(
                        worker,
                        "Finished syncing namespace {}, current progress: {} groups, {} snapshots",
                        namespace,
                        groups,
                        snapshots,
                    );
                }
            }
            Err(err) => {
                errors = true;
                task_log!(
                    worker,
                    "Encountered errors while syncing namespace {} - {}",
                    namespace,
                    err,
                );
            }
        };
    }

    if params.remove_vanished {
        errors |= check_and_remove_vanished_ns(worker, params, synced_ns)?;
    }

    if errors {
        bail!("sync failed with some errors.");
    }

    Ok(())
}

/// Pulls a namespace according to `params`.
///
/// Pulling a namespace consists of the following steps:
/// - Query list of groups on the remote (in `source_ns`)
/// - Filter list according to configured group filters
/// - Iterate list and attempt to pull each group in turn
/// - (remove_vanished) remove groups with matching owner and matching the configured group filters which are
///   not or no longer available on the remote
///
/// Permission checks:
/// - remote namespaces are filtered by remote
/// - owner check for vanished groups done here
pub async fn pull_ns(
    worker: &WorkerTask,
    client: &HttpClient,
    params: &PullParameters,
    source_ns: BackupNamespace,
    target_ns: BackupNamespace,
) -> Result<(StoreProgress, bool), Error> {
    let path = format!("api2/json/admin/datastore/{}/groups", params.source.store());

    let args = if !source_ns.is_root() {
        Some(json!({
            "backup-ns": source_ns,
        }))
    } else {
        None
    };

    let mut result = client
        .get(&path, args)
        .await
        .map_err(|err| format_err!("Failed to retrieve backup groups from remote - {}", err))?;

    let mut list: Vec<GroupListItem> = serde_json::from_value(result["data"].take())?;

    let total_count = list.len();
    list.sort_unstable_by(|a, b| {
        let type_order = a.backup.ty.cmp(&b.backup.ty);
        if type_order == std::cmp::Ordering::Equal {
            a.backup.id.cmp(&b.backup.id)
        } else {
            type_order
        }
    });

    let apply_filters = |group: &pbs_api_types::BackupGroup, filters: &[GroupFilter]| -> bool {
        filters.iter().any(|filter| group.matches(filter))
    };

    // Get groups with target NS set
    let list: Vec<pbs_api_types::BackupGroup> = list.into_iter().map(|item| item.backup).collect();

    let list = if let Some(ref group_filter) = &params.group_filter {
        let unfiltered_count = list.len();
        let list: Vec<pbs_api_types::BackupGroup> = list
            .into_iter()
            .filter(|group| apply_filters(group, group_filter))
            .collect();
        task_log!(
            worker,
            "found {} groups to sync (out of {} total)",
            list.len(),
            unfiltered_count
        );
        list
    } else {
        task_log!(worker, "found {} groups to sync", total_count);
        list
    };

    let mut errors = false;

    let mut new_groups = std::collections::HashSet::new();
    for group in list.iter() {
        new_groups.insert(group.clone());
    }

    let mut progress = StoreProgress::new(list.len() as u64);

    for (done, group) in list.into_iter().enumerate() {
        progress.done_groups = done as u64;
        progress.done_snapshots = 0;
        progress.group_snapshots = 0;

        let (owner, _lock_guard) =
            match params
                .store
                .create_locked_backup_group(&target_ns, &group, &params.owner)
            {
                Ok(result) => result,
                Err(err) => {
                    task_log!(
                        worker,
                        "sync group {} failed - group lock failed: {}",
                        &group,
                        err
                    );
                    errors = true; // do not stop here, instead continue
                    continue;
                }
            };

        // permission check
        if params.owner != owner {
            // only the owner is allowed to create additional snapshots
            task_log!(
                worker,
                "sync group {} failed - owner check failed ({} != {})",
                &group,
                params.owner,
                owner
            );
            errors = true; // do not stop here, instead continue
        } else if let Err(err) = pull_group(
            worker,
            client,
            params,
            &group,
            source_ns.clone(),
            &mut progress,
        )
        .await
        {
            task_log!(worker, "sync group {} failed - {}", &group, err,);
            errors = true; // do not stop here, instead continue
        }
    }

    if params.remove_vanished {
        let result: Result<(), Error> = proxmox_lang::try_block!({
            for local_group in params.store.iter_backup_groups(target_ns.clone())? {
                let local_group = local_group?;
                if new_groups.contains(local_group.as_ref()) {
                    continue;
                }
                let owner = params.store.get_owner(&target_ns, local_group.group())?;
                if check_backup_owner(&owner, &params.owner).is_err() {
                    continue;
                }
                if let Some(ref group_filter) = &params.group_filter {
                    if !apply_filters(local_group.as_ref(), group_filter) {
                        continue;
                    }
                }
                task_log!(
                    worker,
                    "delete vanished group '{}/{}'",
                    local_group.backup_type(),
                    local_group.backup_id()
                );
                match params
                    .store
                    .remove_backup_group(&target_ns, local_group.as_ref())
                {
                    Ok(true) => {}
                    Ok(false) => {
                        task_log!(
                            worker,
                            "kept some protected snapshots of group '{}'",
                            local_group
                        );
                    }
                    Err(err) => {
                        task_log!(worker, "{}", err);
                        errors = true;
                    }
                }
            }
            Ok(())
        });
        if let Err(err) = result {
            task_log!(worker, "error during cleanup: {}", err);
            errors = true;
        };
    }

    Ok((progress, errors))
}
