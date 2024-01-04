//! Sync datastore from remote server

use std::collections::{HashMap, HashSet};
use std::io::{Seek, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use anyhow::{bail, format_err, Error};
use http::StatusCode;
use proxmox_rest_server::WorkerTask;
use proxmox_router::HttpError;
use proxmox_sys::{task_log, task_warn};
use serde_json::json;

use pbs_api_types::{
    print_store_and_ns, Authid, BackupDir, BackupGroup, BackupNamespace, CryptMode, GroupFilter,
    GroupListItem, Operation, RateLimitConfig, Remote, SnapshotListItem, MAX_NAMESPACE_DEPTH,
    PRIV_DATASTORE_AUDIT, PRIV_DATASTORE_BACKUP, PRIV_DATASTORE_READ,
};
use pbs_client::{BackupReader, BackupRepository, HttpClient, RemoteChunkReader};
use pbs_config::CachedUserInfo;
use pbs_datastore::data_blob::DataBlob;
use pbs_datastore::dynamic_index::DynamicIndexReader;
use pbs_datastore::fixed_index::FixedIndexReader;
use pbs_datastore::index::IndexFile;
use pbs_datastore::manifest::{
    archive_type, ArchiveType, BackupManifest, FileInfo, CLIENT_LOG_BLOB_NAME, MANIFEST_BLOB_NAME,
};
use pbs_datastore::read_chunk::AsyncReadChunk;
use pbs_datastore::{
    check_backup_owner, DataStore, ListNamespacesRecursive, LocalChunkReader, StoreProgress,
};
use pbs_tools::sha::sha256;

use crate::backup::{check_ns_modification_privs, check_ns_privs, ListAccessibleBackupGroups};
use crate::tools::parallel_handler::ParallelHandler;

struct RemoteReader {
    backup_reader: Arc<BackupReader>,
    dir: BackupDir,
}

struct LocalReader {
    _dir_lock: Arc<Mutex<proxmox_sys::fs::DirLockGuard>>,
    path: PathBuf,
    datastore: Arc<DataStore>,
}

pub(crate) struct PullTarget {
    store: Arc<DataStore>,
    ns: BackupNamespace,
}

pub(crate) struct RemoteSource {
    repo: BackupRepository,
    ns: BackupNamespace,
    client: HttpClient,
}

pub(crate) struct LocalSource {
    store: Arc<DataStore>,
    ns: BackupNamespace,
}

#[async_trait::async_trait]
/// `PullSource` is a trait that provides an interface for pulling data/information from a source.
/// The trait includes methods for listing namespaces, groups, and backup directories,
/// as well as retrieving a reader for reading data from the source
trait PullSource: Send + Sync {
    /// Lists namespaces from the source.
    async fn list_namespaces(
        &self,
        max_depth: &mut Option<usize>,
        worker: &WorkerTask,
    ) -> Result<Vec<BackupNamespace>, Error>;

    /// Lists groups within a specific namespace from the source.
    async fn list_groups(
        &self,
        namespace: &BackupNamespace,
        owner: &Authid,
    ) -> Result<Vec<BackupGroup>, Error>;

    /// Lists backup directories for a specific group within a specific namespace from the source.
    async fn list_backup_dirs(
        &self,
        namespace: &BackupNamespace,
        group: &BackupGroup,
        worker: &WorkerTask,
    ) -> Result<Vec<BackupDir>, Error>;
    fn get_ns(&self) -> BackupNamespace;
    fn get_store(&self) -> &str;

    /// Returns a reader for reading data from a specific backup directory.
    async fn reader(
        &self,
        ns: &BackupNamespace,
        dir: &BackupDir,
    ) -> Result<Arc<dyn PullReader>, Error>;
}

#[async_trait::async_trait]
impl PullSource for RemoteSource {
    async fn list_namespaces(
        &self,
        max_depth: &mut Option<usize>,
        worker: &WorkerTask,
    ) -> Result<Vec<BackupNamespace>, Error> {
        if self.ns.is_root() && max_depth.map_or(false, |depth| depth == 0) {
            return Ok(vec![self.ns.clone()]);
        }

        let path = format!("api2/json/admin/datastore/{}/namespace", self.repo.store());
        let mut data = json!({});
        if let Some(max_depth) = max_depth {
            data["max-depth"] = json!(max_depth);
        }

        if !self.ns.is_root() {
            data["parent"] = json!(self.ns);
        }
        self.client.login().await?;

        let mut result = match self.client.get(&path, Some(data)).await {
            Ok(res) => res,
            Err(err) => match err.downcast_ref::<HttpError>() {
                Some(HttpError { code, message }) => match code {
                    &StatusCode::NOT_FOUND => {
                        if self.ns.is_root() && max_depth.is_none() {
                            task_warn!(worker, "Could not query remote for namespaces (404) -> temporarily switching to backwards-compat mode");
                            task_warn!(worker, "Either make backwards-compat mode explicit (max-depth == 0) or upgrade remote system.");
                            max_depth.replace(0);
                        } else {
                            bail!("Remote namespace set/recursive sync requested, but remote does not support namespaces.")
                        }

                        return Ok(vec![self.ns.clone()]);
                    }
                    _ => {
                        bail!("Querying namespaces failed - HTTP error {code} - {message}");
                    }
                },
                None => {
                    bail!("Querying namespaces failed - {err}");
                }
            },
        };

        let list: Vec<BackupNamespace> =
            serde_json::from_value::<Vec<pbs_api_types::NamespaceListItem>>(result["data"].take())?
                .into_iter()
                .map(|list_item| list_item.ns)
                .collect();

        Ok(list)
    }

    async fn list_groups(
        &self,
        namespace: &BackupNamespace,
        _owner: &Authid,
    ) -> Result<Vec<BackupGroup>, Error> {
        let path = format!("api2/json/admin/datastore/{}/groups", self.repo.store());

        let args = if !namespace.is_root() {
            Some(json!({ "ns": namespace.clone() }))
        } else {
            None
        };

        self.client.login().await?;
        let mut result =
            self.client.get(&path, args).await.map_err(|err| {
                format_err!("Failed to retrieve backup groups from remote - {}", err)
            })?;

        Ok(
            serde_json::from_value::<Vec<GroupListItem>>(result["data"].take())
                .map_err(Error::from)?
                .into_iter()
                .map(|item| item.backup)
                .collect::<Vec<BackupGroup>>(),
        )
    }

    async fn list_backup_dirs(
        &self,
        namespace: &BackupNamespace,
        group: &BackupGroup,
        worker: &WorkerTask,
    ) -> Result<Vec<BackupDir>, Error> {
        let path = format!("api2/json/admin/datastore/{}/snapshots", self.repo.store());

        let mut args = json!({
            "backup-type": group.ty,
            "backup-id": group.id,
        });

        if !namespace.is_root() {
            args["ns"] = serde_json::to_value(&namespace)?;
        }

        self.client.login().await?;

        let mut result = self.client.get(&path, Some(args)).await?;
        let snapshot_list: Vec<SnapshotListItem> = serde_json::from_value(result["data"].take())?;
        Ok(snapshot_list
            .into_iter()
            .filter_map(|item: SnapshotListItem| {
                let snapshot = item.backup;
                // in-progress backups can't be synced
                if item.size.is_none() {
                    task_log!(
                        worker,
                        "skipping snapshot {} - in-progress backup",
                        snapshot
                    );
                    return None;
                }

                Some(snapshot)
            })
            .collect::<Vec<BackupDir>>())
    }

    fn get_ns(&self) -> BackupNamespace {
        self.ns.clone()
    }

    fn get_store(&self) -> &str {
        &self.repo.store()
    }

    async fn reader(
        &self,
        ns: &BackupNamespace,
        dir: &BackupDir,
    ) -> Result<Arc<dyn PullReader>, Error> {
        let backup_reader =
            BackupReader::start(&self.client, None, self.repo.store(), ns, dir, true).await?;
        Ok(Arc::new(RemoteReader {
            backup_reader,
            dir: dir.clone(),
        }))
    }
}

#[async_trait::async_trait]
impl PullSource for LocalSource {
    async fn list_namespaces(
        &self,
        max_depth: &mut Option<usize>,
        _worker: &WorkerTask,
    ) -> Result<Vec<BackupNamespace>, Error> {
        ListNamespacesRecursive::new_max_depth(
            self.store.clone(),
            self.ns.clone(),
            max_depth.unwrap_or(MAX_NAMESPACE_DEPTH),
        )?
        .collect()
    }

    async fn list_groups(
        &self,
        namespace: &BackupNamespace,
        owner: &Authid,
    ) -> Result<Vec<BackupGroup>, Error> {
        Ok(ListAccessibleBackupGroups::new_with_privs(
            &self.store,
            namespace.clone(),
            0,
            Some(PRIV_DATASTORE_READ),
            Some(PRIV_DATASTORE_BACKUP),
            Some(owner),
        )?
        .filter_map(Result::ok)
        .map(|backup_group| backup_group.group().clone())
        .collect::<Vec<pbs_api_types::BackupGroup>>())
    }

    async fn list_backup_dirs(
        &self,
        namespace: &BackupNamespace,
        group: &BackupGroup,
        _worker: &WorkerTask,
    ) -> Result<Vec<BackupDir>, Error> {
        Ok(self
            .store
            .backup_group(namespace.clone(), group.clone())
            .iter_snapshots()?
            .filter_map(Result::ok)
            .map(|snapshot| snapshot.dir().to_owned())
            .collect::<Vec<BackupDir>>())
    }

    fn get_ns(&self) -> BackupNamespace {
        self.ns.clone()
    }

    fn get_store(&self) -> &str {
        self.store.name()
    }

    async fn reader(
        &self,
        ns: &BackupNamespace,
        dir: &BackupDir,
    ) -> Result<Arc<dyn PullReader>, Error> {
        let dir = self.store.backup_dir(ns.clone(), dir.clone())?;
        let dir_lock = proxmox_sys::fs::lock_dir_noblock_shared(
            &dir.full_path(),
            "snapshot",
            "locked by another operation",
        )?;
        Ok(Arc::new(LocalReader {
            _dir_lock: Arc::new(Mutex::new(dir_lock)),
            path: dir.full_path(),
            datastore: dir.datastore().clone(),
        }))
    }
}

#[async_trait::async_trait]
/// `PullReader` is a trait that provides an interface for reading data from a source.
/// The trait includes methods for getting a chunk reader, loading a file, downloading client log, and checking whether chunk sync should be skipped.
trait PullReader: Send + Sync {
    /// Returns a chunk reader with the specified encryption mode.
    fn chunk_reader(&self, crypt_mode: CryptMode) -> Arc<dyn AsyncReadChunk>;

    /// Asynchronously loads a file from the source into a local file.
    /// `filename` is the name of the file to load from the source.
    /// `into` is the path of the local file to load the source file into.
    async fn load_file_into(
        &self,
        filename: &str,
        into: &Path,
        worker: &WorkerTask,
    ) -> Result<Option<DataBlob>, Error>;

    /// Tries to download the client log from the source and save it into a local file.
    async fn try_download_client_log(
        &self,
        to_path: &Path,
        worker: &WorkerTask,
    ) -> Result<(), Error>;

    fn skip_chunk_sync(&self, target_store_name: &str) -> bool;
}

#[async_trait::async_trait]
impl PullReader for RemoteReader {
    fn chunk_reader(&self, crypt_mode: CryptMode) -> Arc<dyn AsyncReadChunk> {
        Arc::new(RemoteChunkReader::new(
            self.backup_reader.clone(),
            None,
            crypt_mode,
            HashMap::new(),
        ))
    }

    async fn load_file_into(
        &self,
        filename: &str,
        into: &Path,
        worker: &WorkerTask,
    ) -> Result<Option<DataBlob>, Error> {
        let mut tmp_file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .read(true)
            .open(into)?;
        let download_result = self.backup_reader.download(filename, &mut tmp_file).await;
        if let Err(err) = download_result {
            match err.downcast_ref::<HttpError>() {
                Some(HttpError { code, message }) => match *code {
                    StatusCode::NOT_FOUND => {
                        task_log!(
                            worker,
                            "skipping snapshot {} - vanished since start of sync",
                            &self.dir,
                        );
                        return Ok(None);
                    }
                    _ => {
                        bail!("HTTP error {code} - {message}");
                    }
                },
                None => {
                    return Err(err);
                }
            };
        };
        tmp_file.rewind()?;
        Ok(DataBlob::load_from_reader(&mut tmp_file).ok())
    }

    async fn try_download_client_log(
        &self,
        to_path: &Path,
        worker: &WorkerTask,
    ) -> Result<(), Error> {
        let mut tmp_path = to_path.to_owned();
        tmp_path.set_extension("tmp");

        let tmpfile = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .read(true)
            .open(&tmp_path)?;

        // Note: be silent if there is no log - only log successful download
        if let Ok(()) = self
            .backup_reader
            .download(CLIENT_LOG_BLOB_NAME, tmpfile)
            .await
        {
            if let Err(err) = std::fs::rename(&tmp_path, to_path) {
                bail!("Atomic rename file {:?} failed - {}", to_path, err);
            }
            task_log!(worker, "got backup log file {:?}", CLIENT_LOG_BLOB_NAME);
        }

        Ok(())
    }

    fn skip_chunk_sync(&self, _target_store_name: &str) -> bool {
        false
    }
}

#[async_trait::async_trait]
impl PullReader for LocalReader {
    fn chunk_reader(&self, crypt_mode: CryptMode) -> Arc<dyn AsyncReadChunk> {
        Arc::new(LocalChunkReader::new(
            self.datastore.clone(),
            None,
            crypt_mode,
        ))
    }

    async fn load_file_into(
        &self,
        filename: &str,
        into: &Path,
        _worker: &WorkerTask,
    ) -> Result<Option<DataBlob>, Error> {
        let mut tmp_file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .read(true)
            .open(into)?;
        let mut from_path = self.path.clone();
        from_path.push(filename);
        tmp_file.write_all(std::fs::read(from_path)?.as_slice())?;
        tmp_file.rewind()?;
        Ok(DataBlob::load_from_reader(&mut tmp_file).ok())
    }

    async fn try_download_client_log(
        &self,
        _to_path: &Path,
        _worker: &WorkerTask,
    ) -> Result<(), Error> {
        Ok(())
    }

    fn skip_chunk_sync(&self, target_store_name: &str) -> bool {
        self.datastore.name() == target_store_name
    }
}

/// Parameters for a pull operation.
pub(crate) struct PullParameters {
    /// Where data is pulled from
    source: Arc<dyn PullSource>,
    /// Where data should be pulled into
    target: PullTarget,
    /// Owner of synced groups (needs to match local owner of pre-existing groups)
    owner: Authid,
    /// Whether to remove groups which exist locally, but not on the remote end
    remove_vanished: bool,
    /// How many levels of sub-namespaces to pull (0 == no recursion, None == maximum recursion)
    max_depth: Option<usize>,
    /// Filters for reducing the pull scope
    group_filter: Vec<GroupFilter>,
    /// How many snapshots should be transferred at most (taking the newest N snapshots)
    transfer_last: Option<usize>,
}

impl PullParameters {
    /// Creates a new instance of `PullParameters`.
    pub(crate) fn new(
        store: &str,
        ns: BackupNamespace,
        remote: Option<&str>,
        remote_store: &str,
        remote_ns: BackupNamespace,
        owner: Authid,
        remove_vanished: Option<bool>,
        max_depth: Option<usize>,
        group_filter: Option<Vec<GroupFilter>>,
        limit: RateLimitConfig,
        transfer_last: Option<usize>,
    ) -> Result<Self, Error> {
        if let Some(max_depth) = max_depth {
            ns.check_max_depth(max_depth)?;
            remote_ns.check_max_depth(max_depth)?;
        };
        let remove_vanished = remove_vanished.unwrap_or(false);

        let source: Arc<dyn PullSource> = if let Some(remote) = remote {
            let (remote_config, _digest) = pbs_config::remote::config()?;
            let remote: Remote = remote_config.lookup("remote", remote)?;

            let repo = BackupRepository::new(
                Some(remote.config.auth_id.clone()),
                Some(remote.config.host.clone()),
                remote.config.port,
                remote_store.to_string(),
            );
            let client = crate::api2::config::remote::remote_client_config(&remote, Some(limit))?;
            Arc::new(RemoteSource {
                repo,
                ns: remote_ns,
                client,
            })
        } else {
            Arc::new(LocalSource {
                store: DataStore::lookup_datastore(remote_store, Some(Operation::Read))?,
                ns: remote_ns,
            })
        };
        let target = PullTarget {
            store: DataStore::lookup_datastore(store, Some(Operation::Write))?,
            ns,
        };

        let group_filter = group_filter.unwrap_or_default();

        Ok(Self {
            source,
            target,
            owner,
            remove_vanished,
            max_depth,
            group_filter,
            transfer_last,
        })
    }
}

async fn pull_index_chunks<I: IndexFile>(
    worker: &WorkerTask,
    chunk_reader: Arc<dyn AsyncReadChunk>,
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
/// - Load archive file into tmp file
/// -- Load file into tmp file
/// -- Verify tmp file checksum
/// - if archive is an index, pull referenced chunks
/// - Rename tmp file into real path
async fn pull_single_archive<'a>(
    worker: &'a WorkerTask,
    reader: Arc<dyn PullReader + 'a>,
    snapshot: &'a pbs_datastore::BackupDir,
    archive_info: &'a FileInfo,
    downloaded_chunks: Arc<Mutex<HashSet<[u8; 32]>>>,
) -> Result<(), Error> {
    let archive_name = &archive_info.filename;
    let mut path = snapshot.full_path();
    path.push(archive_name);

    let mut tmp_path = path.clone();
    tmp_path.set_extension("tmp");

    task_log!(worker, "sync archive {}", archive_name);

    reader
        .load_file_into(archive_name, &tmp_path, worker)
        .await?;

    let mut tmpfile = std::fs::OpenOptions::new().read(true).open(&tmp_path)?;

    match archive_type(archive_name)? {
        ArchiveType::DynamicIndex => {
            let index = DynamicIndexReader::new(tmpfile).map_err(|err| {
                format_err!("unable to read dynamic index {:?} - {}", tmp_path, err)
            })?;
            let (csum, size) = index.compute_csum();
            verify_archive(archive_info, &csum, size)?;

            if reader.skip_chunk_sync(snapshot.datastore().name()) {
                task_log!(worker, "skipping chunk sync for same datastore");
            } else {
                pull_index_chunks(
                    worker,
                    reader.chunk_reader(archive_info.crypt_mode),
                    snapshot.datastore().clone(),
                    index,
                    downloaded_chunks,
                )
                .await?;
            }
        }
        ArchiveType::FixedIndex => {
            let index = FixedIndexReader::new(tmpfile).map_err(|err| {
                format_err!("unable to read fixed index '{:?}' - {}", tmp_path, err)
            })?;
            let (csum, size) = index.compute_csum();
            verify_archive(archive_info, &csum, size)?;

            if reader.skip_chunk_sync(snapshot.datastore().name()) {
                task_log!(worker, "skipping chunk sync for same datastore");
            } else {
                pull_index_chunks(
                    worker,
                    reader.chunk_reader(archive_info.crypt_mode),
                    snapshot.datastore().clone(),
                    index,
                    downloaded_chunks,
                )
                .await?;
            }
        }
        ArchiveType::Blob => {
            tmpfile.rewind()?;
            let (csum, size) = sha256(&mut tmpfile)?;
            verify_archive(archive_info, &csum, size)?;
        }
    }
    if let Err(err) = std::fs::rename(&tmp_path, &path) {
        bail!("Atomic rename file {:?} failed - {}", path, err);
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
async fn pull_snapshot<'a>(
    worker: &'a WorkerTask,
    reader: Arc<dyn PullReader + 'a>,
    snapshot: &'a pbs_datastore::BackupDir,
    downloaded_chunks: Arc<Mutex<HashSet<[u8; 32]>>>,
) -> Result<(), Error> {
    let mut manifest_name = snapshot.full_path();
    manifest_name.push(MANIFEST_BLOB_NAME);

    let mut client_log_name = snapshot.full_path();
    client_log_name.push(CLIENT_LOG_BLOB_NAME);

    let mut tmp_manifest_name = manifest_name.clone();
    tmp_manifest_name.set_extension("tmp");
    let tmp_manifest_blob;
    if let Some(data) = reader
        .load_file_into(MANIFEST_BLOB_NAME, &tmp_manifest_name, worker)
        .await?
    {
        tmp_manifest_blob = data;
    } else {
        return Ok(());
    }

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
                reader
                    .try_download_client_log(&client_log_name, worker)
                    .await?;
            };
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

        pull_single_archive(
            worker,
            reader.clone(),
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
        reader
            .try_download_client_log(&client_log_name, worker)
            .await?;
    };
    snapshot
        .cleanup_unreferenced_files(&manifest)
        .map_err(|err| format_err!("failed to cleanup unreferenced files - {err}"))?;

    Ok(())
}

/// Pulls a `snapshot`, removing newly created ones on error, but keeping existing ones in any case.
///
/// The `reader` is configured to read from the source backup directory, while the
/// `snapshot` is pointing to the local datastore and target namespace.
async fn pull_snapshot_from<'a>(
    worker: &'a WorkerTask,
    reader: Arc<dyn PullReader + 'a>,
    snapshot: &'a pbs_datastore::BackupDir,
    downloaded_chunks: Arc<Mutex<HashSet<[u8; 32]>>>,
) -> Result<(), Error> {
    let (_path, is_new, _snap_lock) = snapshot
        .datastore()
        .create_locked_backup_dir(snapshot.backup_ns(), snapshot.as_ref())?;

    if is_new {
        task_log!(worker, "sync snapshot {}", snapshot.dir());

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
        task_log!(worker, "sync snapshot {} done", snapshot.dir());
    } else {
        task_log!(worker, "re-sync snapshot {}", snapshot.dir());
        pull_snapshot(worker, reader, snapshot, downloaded_chunks).await?;
    }

    Ok(())
}

#[derive(PartialEq, Eq)]
enum SkipReason {
    AlreadySynced,
    TransferLast,
}

impl std::fmt::Display for SkipReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                SkipReason::AlreadySynced => "older than the newest local snapshot",
                SkipReason::TransferLast => "due to transfer-last",
            }
        )
    }
}

struct SkipInfo {
    oldest: i64,
    newest: i64,
    count: u64,
    skip_reason: SkipReason,
}

impl SkipInfo {
    fn new(skip_reason: SkipReason) -> Self {
        SkipInfo {
            oldest: i64::MAX,
            newest: i64::MIN,
            count: 0,
            skip_reason,
        }
    }

    fn reset(&mut self) {
        self.count = 0;
        self.oldest = i64::MAX;
        self.newest = i64::MIN;
    }

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
            "skipped: {} snapshot(s) ({}) - {}",
            self.count,
            self.affected().map_err(|_| std::fmt::Error)?,
            self.skip_reason,
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
/// -- pull snapshot, unless it's not finished yet or older than last local snapshot
/// - (remove_vanished) list all local snapshots, remove those that don't exist on remote
///
/// Backwards-compat: if `source_namespace` is [None], only the group type and ID will be sent to the
/// remote when querying snapshots. This allows us to interact with old remotes that don't have
/// namespace support yet.
///
/// Permission checks:
/// - remote snapshot access is checked by remote (twice: query and opening the backup reader)
/// - local group owner is already checked by pull_store
async fn pull_group(
    worker: &WorkerTask,
    params: &PullParameters,
    source_namespace: &BackupNamespace,
    group: &BackupGroup,
    progress: &mut StoreProgress,
) -> Result<(), Error> {
    let mut already_synced_skip_info = SkipInfo::new(SkipReason::AlreadySynced);
    let mut transfer_last_skip_info = SkipInfo::new(SkipReason::TransferLast);

    let mut raw_list: Vec<BackupDir> = params
        .source
        .list_backup_dirs(source_namespace, group, worker)
        .await?;
    raw_list.sort_unstable_by(|a, b| a.time.cmp(&b.time));

    let total_amount = raw_list.len();

    let cutoff = params
        .transfer_last
        .map(|count| total_amount.saturating_sub(count))
        .unwrap_or_default();

    let target_ns = source_namespace.map_prefix(&params.source.get_ns(), &params.target.ns)?;

    let mut source_snapshots = HashSet::new();
    let last_sync_time = params
        .target
        .store
        .last_successful_backup(&target_ns, group)?
        .unwrap_or(i64::MIN);

    let list: Vec<BackupDir> = raw_list
        .into_iter()
        .enumerate()
        .filter(|&(pos, ref dir)| {
            source_snapshots.insert(dir.time);
            if last_sync_time > dir.time {
                already_synced_skip_info.update(dir.time);
                return false;
            } else if already_synced_skip_info.count > 0 {
                task_log!(worker, "{}", already_synced_skip_info);
                already_synced_skip_info.reset();
                return true;
            }

            if pos < cutoff && last_sync_time != dir.time {
                transfer_last_skip_info.update(dir.time);
                return false;
            } else if transfer_last_skip_info.count > 0 {
                task_log!(worker, "{}", transfer_last_skip_info);
                transfer_last_skip_info.reset();
            }
            true
        })
        .map(|(_, dir)| dir)
        .collect();

    // start with 65536 chunks (up to 256 GiB)
    let downloaded_chunks = Arc::new(Mutex::new(HashSet::with_capacity(1024 * 64)));

    progress.group_snapshots = list.len() as u64;

    for (pos, from_snapshot) in list.into_iter().enumerate() {
        let to_snapshot = params
            .target
            .store
            .backup_dir(target_ns.clone(), from_snapshot.clone())?;

        let reader = params
            .source
            .reader(source_namespace, &from_snapshot)
            .await?;
        let result =
            pull_snapshot_from(worker, reader, &to_snapshot, downloaded_chunks.clone()).await;

        progress.done_snapshots = pos as u64 + 1;
        task_log!(worker, "percentage done: {}", progress);

        result?; // stop on error
    }

    if params.remove_vanished {
        let group = params
            .target
            .store
            .backup_group(target_ns.clone(), group.clone());
        let local_list = group.list_backups()?;
        for info in local_list {
            let snapshot = info.backup_dir;
            if source_snapshots.contains(&snapshot.backup_time()) {
                continue;
            }
            if snapshot.is_protected() {
                task_log!(
                    worker,
                    "don't delete vanished snapshot {} (protected)",
                    snapshot.dir()
                );
                continue;
            }
            task_log!(worker, "delete vanished snapshot {}", snapshot.dir());
            params
                .target
                .store
                .remove_backup_dir(&target_ns, snapshot.as_ref(), false)?;
        }
    }

    Ok(())
}

fn check_and_create_ns(params: &PullParameters, ns: &BackupNamespace) -> Result<bool, Error> {
    let mut created = false;
    let store_ns_str = print_store_and_ns(params.target.store.name(), ns);

    if !ns.is_root() && !params.target.store.namespace_path(ns).exists() {
        check_ns_modification_privs(params.target.store.name(), ns, &params.owner)
            .map_err(|err| format_err!("Creating {ns} not allowed - {err}"))?;

        let name = match ns.components().last() {
            Some(name) => name.to_owned(),
            None => {
                bail!("Failed to determine last component of namespace.");
            }
        };

        if let Err(err) = params.target.store.create_namespace(&ns.parent(), name) {
            bail!("sync into {store_ns_str} failed - namespace creation failed: {err}");
        }
        created = true;
    }

    check_ns_privs(
        params.target.store.name(),
        ns,
        &params.owner,
        PRIV_DATASTORE_BACKUP,
    )
    .map_err(|err| format_err!("sync into {store_ns_str} not allowed - {err}"))?;

    Ok(created)
}

fn check_and_remove_ns(params: &PullParameters, local_ns: &BackupNamespace) -> Result<bool, Error> {
    check_ns_modification_privs(params.target.store.name(), local_ns, &params.owner)
        .map_err(|err| format_err!("Removing {local_ns} not allowed - {err}"))?;

    params
        .target
        .store
        .remove_namespace_recursive(local_ns, true)
}

fn check_and_remove_vanished_ns(
    worker: &WorkerTask,
    params: &PullParameters,
    synced_ns: HashSet<BackupNamespace>,
) -> Result<bool, Error> {
    let mut errors = false;
    let user_info = CachedUserInfo::new()?;

    // clamp like remote does so that we don't list more than we can ever have synced.
    let max_depth = params
        .max_depth
        .unwrap_or_else(|| MAX_NAMESPACE_DEPTH - params.source.get_ns().depth());

    let mut local_ns_list: Vec<BackupNamespace> = params
        .target
        .store
        .recursive_iter_backup_ns_ok(params.target.ns.clone(), Some(max_depth))?
        .filter(|ns| {
            let user_privs =
                user_info.lookup_privs(&params.owner, &ns.acl_path(params.target.store.name()));
            user_privs & (PRIV_DATASTORE_BACKUP | PRIV_DATASTORE_AUDIT) != 0
        })
        .collect();

    // children first!
    local_ns_list.sort_unstable_by_key(|b| std::cmp::Reverse(b.name_len()));

    for local_ns in local_ns_list {
        if local_ns == params.target.ns {
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
pub(crate) async fn pull_store(
    worker: &WorkerTask,
    mut params: PullParameters,
) -> Result<(), Error> {
    // explicit create shared lock to prevent GC on newly created chunks
    let _shared_store_lock = params.target.store.try_shared_chunk_store_lock()?;
    let mut errors = false;

    let old_max_depth = params.max_depth;
    let mut namespaces = if params.source.get_ns().is_root() && old_max_depth == Some(0) {
        vec![params.source.get_ns()] // backwards compat - don't query remote namespaces!
    } else {
        params
            .source
            .list_namespaces(&mut params.max_depth, worker)
            .await?
    };

    let ns_layers_to_be_pulled = namespaces
        .iter()
        .map(BackupNamespace::depth)
        .max()
        .map_or(0, |v| v - params.source.get_ns().depth());
    let target_depth = params.target.ns.depth();

    if ns_layers_to_be_pulled + target_depth > MAX_NAMESPACE_DEPTH {
        bail!(
            "Syncing would exceed max allowed namespace depth. ({}+{} > {})",
            ns_layers_to_be_pulled,
            target_depth,
            MAX_NAMESPACE_DEPTH
        );
    }

    errors |= old_max_depth != params.max_depth; // fail job if we switched to backwards-compat mode
    namespaces.sort_unstable_by_key(|a| a.name_len());

    let (mut groups, mut snapshots) = (0, 0);
    let mut synced_ns = HashSet::with_capacity(namespaces.len());

    for namespace in namespaces {
        let source_store_ns_str = print_store_and_ns(params.source.get_store(), &namespace);

        let target_ns = namespace.map_prefix(&params.source.get_ns(), &params.target.ns)?;
        let target_store_ns_str = print_store_and_ns(params.target.store.name(), &target_ns);

        task_log!(worker, "----");
        task_log!(
            worker,
            "Syncing {} into {}",
            source_store_ns_str,
            target_store_ns_str
        );

        synced_ns.insert(target_ns.clone());

        match check_and_create_ns(&params, &target_ns) {
            Ok(true) => task_log!(worker, "Created namespace {}", target_ns),
            Ok(false) => {}
            Err(err) => {
                task_log!(
                    worker,
                    "Cannot sync {} into {} - {}",
                    source_store_ns_str,
                    target_store_ns_str,
                    err,
                );
                errors = true;
                continue;
            }
        }

        match pull_ns(worker, &namespace, &mut params).await {
            Ok((ns_progress, ns_errors)) => {
                errors |= ns_errors;

                if params.max_depth != Some(0) {
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
                    &namespace,
                    err,
                );
            }
        };
    }

    if params.remove_vanished {
        errors |= check_and_remove_vanished_ns(worker, &params, synced_ns)?;
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
pub(crate) async fn pull_ns(
    worker: &WorkerTask,
    namespace: &BackupNamespace,
    params: &mut PullParameters,
) -> Result<(StoreProgress, bool), Error> {
    let mut list: Vec<BackupGroup> = params.source.list_groups(namespace, &params.owner).await?;

    list.sort_unstable_by(|a, b| {
        let type_order = a.ty.cmp(&b.ty);
        if type_order == std::cmp::Ordering::Equal {
            a.id.cmp(&b.id)
        } else {
            type_order
        }
    });

    let unfiltered_count = list.len();
    let list: Vec<BackupGroup> = list
        .into_iter()
        .filter(|group| group.apply_filters(&params.group_filter))
        .collect();
    task_log!(
        worker,
        "found {} groups to sync (out of {} total)",
        list.len(),
        unfiltered_count
    );

    let mut errors = false;

    let mut new_groups = HashSet::new();
    for group in list.iter() {
        new_groups.insert(group.clone());
    }

    let mut progress = StoreProgress::new(list.len() as u64);

    let target_ns = namespace.map_prefix(&params.source.get_ns(), &params.target.ns)?;

    for (done, group) in list.into_iter().enumerate() {
        progress.done_groups = done as u64;
        progress.done_snapshots = 0;
        progress.group_snapshots = 0;

        let (owner, _lock_guard) =
            match params
                .target
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
                    errors = true;
                    // do not stop here, instead continue
                    task_log!(worker, "create_locked_backup_group failed");
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
        } else if let Err(err) = pull_group(worker, params, namespace, &group, &mut progress).await
        {
            task_log!(worker, "sync group {} failed - {}", &group, err,);
            errors = true; // do not stop here, instead continue
        }
    }

    if params.remove_vanished {
        let result: Result<(), Error> = proxmox_lang::try_block!({
            for local_group in params.target.store.iter_backup_groups(target_ns.clone())? {
                let local_group = local_group?;
                let local_group = local_group.group();
                if new_groups.contains(local_group) {
                    continue;
                }
                let owner = params.target.store.get_owner(&target_ns, local_group)?;
                if check_backup_owner(&owner, &params.owner).is_err() {
                    continue;
                }
                if !local_group.apply_filters(&params.group_filter) {
                    continue;
                }
                task_log!(worker, "delete vanished group '{local_group}'",);
                match params
                    .target
                    .store
                    .remove_backup_group(&target_ns, local_group)
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
