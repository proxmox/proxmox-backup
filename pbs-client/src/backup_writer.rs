use std::collections::HashSet;
use std::future::Future;
use std::os::unix::fs::OpenOptionsExt;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{bail, format_err, Error};
use futures::future::{self, AbortHandle, Either, FutureExt, TryFutureExt};
use futures::stream::{Stream, StreamExt, TryStreamExt};
use serde_json::{json, Value};
use tokio::io::AsyncReadExt;
use tokio::sync::{mpsc, oneshot};
use tokio_stream::wrappers::ReceiverStream;

use pbs_api_types::{BackupDir, BackupNamespace};
use pbs_datastore::data_blob::{ChunkInfo, DataBlob, DataChunkBuilder};
use pbs_datastore::dynamic_index::DynamicIndexReader;
use pbs_datastore::fixed_index::FixedIndexReader;
use pbs_datastore::index::IndexFile;
use pbs_datastore::manifest::{ArchiveType, BackupManifest, MANIFEST_BLOB_NAME};
use pbs_datastore::{CATALOG_NAME, PROXMOX_BACKUP_PROTOCOL_ID_V1};
use pbs_tools::crypt_config::CryptConfig;

use proxmox_human_byte::HumanByte;

use super::merge_known_chunks::{MergeKnownChunks, MergedChunkInfo};

use super::{H2Client, HttpClient};

pub struct BackupWriter {
    h2: H2Client,
    abort: AbortHandle,
    crypt_config: Option<Arc<CryptConfig>>,
}

impl Drop for BackupWriter {
    fn drop(&mut self) {
        self.abort.abort();
    }
}

pub struct BackupStats {
    pub size: u64,
    pub csum: [u8; 32],
}

/// Options for uploading blobs/streams to the server
#[derive(Default, Clone)]
pub struct UploadOptions {
    pub previous_manifest: Option<Arc<BackupManifest>>,
    pub compress: bool,
    pub encrypt: bool,
    pub fixed_size: Option<u64>,
}

struct UploadStats {
    chunk_count: usize,
    chunk_reused: usize,
    size: usize,
    size_reused: usize,
    size_compressed: usize,
    duration: std::time::Duration,
    csum: [u8; 32],
}

type UploadQueueSender = mpsc::Sender<(MergedChunkInfo, Option<h2::client::ResponseFuture>)>;
type UploadResultReceiver = oneshot::Receiver<Result<(), Error>>;

impl BackupWriter {
    fn new(h2: H2Client, abort: AbortHandle, crypt_config: Option<Arc<CryptConfig>>) -> Arc<Self> {
        Arc::new(Self {
            h2,
            abort,
            crypt_config,
        })
    }

    // FIXME: extract into (flattened) parameter struct?
    #[allow(clippy::too_many_arguments)]
    pub async fn start(
        client: HttpClient,
        crypt_config: Option<Arc<CryptConfig>>,
        datastore: &str,
        ns: &BackupNamespace,
        backup: &BackupDir,
        debug: bool,
        benchmark: bool,
    ) -> Result<Arc<BackupWriter>, Error> {
        let mut param = json!({
            "backup-type": backup.ty(),
            "backup-id": backup.id(),
            "backup-time": backup.time,
            "store": datastore,
            "debug": debug,
            "benchmark": benchmark
        });

        if !ns.is_root() {
            param["ns"] = serde_json::to_value(ns)?;
        }

        let req = HttpClient::request_builder(
            client.server(),
            client.port(),
            "GET",
            "/api2/json/backup",
            Some(param),
        )
        .unwrap();

        let (h2, abort) = client
            .start_h2_connection(req, String::from(PROXMOX_BACKUP_PROTOCOL_ID_V1!()))
            .await?;

        Ok(BackupWriter::new(h2, abort, crypt_config))
    }

    pub async fn get(&self, path: &str, param: Option<Value>) -> Result<Value, Error> {
        self.h2.get(path, param).await
    }

    pub async fn put(&self, path: &str, param: Option<Value>) -> Result<Value, Error> {
        self.h2.put(path, param).await
    }

    pub async fn post(&self, path: &str, param: Option<Value>) -> Result<Value, Error> {
        self.h2.post(path, param).await
    }

    pub async fn upload_post(
        &self,
        path: &str,
        param: Option<Value>,
        content_type: &str,
        data: Vec<u8>,
    ) -> Result<Value, Error> {
        self.h2
            .upload("POST", path, param, content_type, data)
            .await
    }

    pub async fn send_upload_request(
        &self,
        method: &str,
        path: &str,
        param: Option<Value>,
        content_type: &str,
        data: Vec<u8>,
    ) -> Result<h2::client::ResponseFuture, Error> {
        let request =
            H2Client::request_builder("localhost", method, path, param, Some(content_type))
                .unwrap();
        let response_future = self
            .h2
            .send_request(request, Some(bytes::Bytes::from(data.clone())))
            .await?;
        Ok(response_future)
    }

    pub async fn upload_put(
        &self,
        path: &str,
        param: Option<Value>,
        content_type: &str,
        data: Vec<u8>,
    ) -> Result<Value, Error> {
        self.h2.upload("PUT", path, param, content_type, data).await
    }

    pub async fn finish(self: Arc<Self>) -> Result<(), Error> {
        let h2 = self.h2.clone();

        h2.post("finish", None)
            .map_ok(move |_| {
                self.abort.abort();
            })
            .await
    }

    pub fn cancel(&self) {
        self.abort.abort();
    }

    pub async fn upload_blob<R: std::io::Read>(
        &self,
        mut reader: R,
        file_name: &str,
    ) -> Result<BackupStats, Error> {
        let mut raw_data = Vec::new();
        // fixme: avoid loading into memory
        reader.read_to_end(&mut raw_data)?;

        let csum = openssl::sha::sha256(&raw_data);
        let param = json!({"encoded-size": raw_data.len(), "file-name": file_name });
        let size = raw_data.len() as u64;
        let _value = self
            .h2
            .upload(
                "POST",
                "blob",
                Some(param),
                "application/octet-stream",
                raw_data,
            )
            .await?;
        Ok(BackupStats { size, csum })
    }

    pub async fn upload_blob_from_data(
        &self,
        data: Vec<u8>,
        file_name: &str,
        options: UploadOptions,
    ) -> Result<BackupStats, Error> {
        let blob = match (options.encrypt, &self.crypt_config) {
            (false, _) => DataBlob::encode(&data, None, options.compress)?,
            (true, None) => bail!("requested encryption without a crypt config"),
            (true, Some(crypt_config)) => {
                DataBlob::encode(&data, Some(crypt_config), options.compress)?
            }
        };

        let raw_data = blob.into_inner();
        let size = raw_data.len() as u64;

        let csum = openssl::sha::sha256(&raw_data);
        let param = json!({"encoded-size": size, "file-name": file_name });
        let _value = self
            .h2
            .upload(
                "POST",
                "blob",
                Some(param),
                "application/octet-stream",
                raw_data,
            )
            .await?;
        Ok(BackupStats { size, csum })
    }

    pub async fn upload_blob_from_file<P: AsRef<std::path::Path>>(
        &self,
        src_path: P,
        file_name: &str,
        options: UploadOptions,
    ) -> Result<BackupStats, Error> {
        let src_path = src_path.as_ref();

        let mut file = tokio::fs::File::open(src_path)
            .await
            .map_err(|err| format_err!("unable to open file {:?} - {}", src_path, err))?;

        let mut contents = Vec::new();

        file.read_to_end(&mut contents)
            .await
            .map_err(|err| format_err!("unable to read file {:?} - {}", src_path, err))?;

        self.upload_blob_from_data(contents, file_name, options)
            .await
    }

    pub async fn upload_stream(
        &self,
        archive_name: &str,
        stream: impl Stream<Item = Result<bytes::BytesMut, Error>>,
        options: UploadOptions,
    ) -> Result<BackupStats, Error> {
        let known_chunks = Arc::new(Mutex::new(HashSet::new()));

        let mut param = json!({ "archive-name": archive_name });
        let prefix = if let Some(size) = options.fixed_size {
            param["size"] = size.into();
            "fixed"
        } else {
            "dynamic"
        };

        if options.encrypt && self.crypt_config.is_none() {
            bail!("requested encryption without a crypt config");
        }

        let index_path = format!("{}_index", prefix);
        let close_path = format!("{}_close", prefix);

        if let Some(manifest) = options.previous_manifest {
            if !manifest
                .files()
                .iter()
                .any(|file| file.filename == archive_name)
            {
                log::info!("Previous manifest does not contain an archive called '{archive_name}', skipping download..");
            } else {
                // try, but ignore errors
                match ArchiveType::from_path(archive_name) {
                    Ok(ArchiveType::FixedIndex) => {
                        if let Err(err) = self
                            .download_previous_fixed_index(
                                archive_name,
                                &manifest,
                                known_chunks.clone(),
                            )
                            .await
                        {
                            log::warn!("Error downloading .fidx from previous manifest: {}", err);
                        }
                    }
                    Ok(ArchiveType::DynamicIndex) => {
                        if let Err(err) = self
                            .download_previous_dynamic_index(
                                archive_name,
                                &manifest,
                                known_chunks.clone(),
                            )
                            .await
                        {
                            log::warn!("Error downloading .didx from previous manifest: {}", err);
                        }
                    }
                    _ => { /* do nothing */ }
                }
            }
        }

        let wid = self
            .h2
            .post(&index_path, Some(param))
            .await?
            .as_u64()
            .unwrap();

        let upload_stats = Self::upload_chunk_info_stream(
            self.h2.clone(),
            wid,
            stream,
            prefix,
            known_chunks.clone(),
            if options.encrypt {
                self.crypt_config.clone()
            } else {
                None
            },
            options.compress,
        )
        .await?;

        let size_dirty = upload_stats.size - upload_stats.size_reused;
        let size: HumanByte = upload_stats.size.into();
        let archive = if log::log_enabled!(log::Level::Debug) {
            archive_name
        } else {
            pbs_tools::format::strip_server_file_extension(archive_name)
        };

        if archive_name != CATALOG_NAME {
            let speed: HumanByte =
                ((size_dirty * 1_000_000) / (upload_stats.duration.as_micros() as usize)).into();
            let size_dirty: HumanByte = size_dirty.into();
            let size_compressed: HumanByte = upload_stats.size_compressed.into();
            log::info!(
                "{}: had to backup {} of {} (compressed {}) in {:.2}s",
                archive,
                size_dirty,
                size,
                size_compressed,
                upload_stats.duration.as_secs_f64()
            );
            log::info!("{}: average backup speed: {}/s", archive, speed);
        } else {
            log::info!("Uploaded backup catalog ({})", size);
        }

        if upload_stats.size_reused > 0 && upload_stats.size > 1024 * 1024 {
            let reused_percent = upload_stats.size_reused as f64 * 100. / upload_stats.size as f64;
            let reused: HumanByte = upload_stats.size_reused.into();
            log::info!(
                "{}: backup was done incrementally, reused {} ({:.1}%)",
                archive,
                reused,
                reused_percent
            );
        }
        if log::log_enabled!(log::Level::Debug) && upload_stats.chunk_count > 0 {
            log::debug!(
                "{}: Reused {} from {} chunks.",
                archive,
                upload_stats.chunk_reused,
                upload_stats.chunk_count
            );
            log::debug!(
                "{}: Average chunk size was {}.",
                archive,
                HumanByte::from(upload_stats.size / upload_stats.chunk_count)
            );
            log::debug!(
                "{}: Average time per request: {} microseconds.",
                archive,
                (upload_stats.duration.as_micros()) / (upload_stats.chunk_count as u128)
            );
        }

        let param = json!({
            "wid": wid ,
            "chunk-count": upload_stats.chunk_count,
            "size": upload_stats.size,
            "csum": hex::encode(upload_stats.csum),
        });
        let _value = self.h2.post(&close_path, Some(param)).await?;
        Ok(BackupStats {
            size: upload_stats.size as u64,
            csum: upload_stats.csum,
        })
    }

    fn response_queue() -> (
        mpsc::Sender<h2::client::ResponseFuture>,
        oneshot::Receiver<Result<(), Error>>,
    ) {
        let (verify_queue_tx, verify_queue_rx) = mpsc::channel(100);
        let (verify_result_tx, verify_result_rx) = oneshot::channel();

        // FIXME: check if this works as expected as replacement for the combinator below?
        // tokio::spawn(async move {
        //     let result: Result<(), Error> = (async move {
        //         while let Some(response) = verify_queue_rx.recv().await {
        //             match H2Client::h2api_response(response.await?).await {
        //                 Ok(result) => println!("RESPONSE: {:?}", result),
        //                 Err(err) => bail!("pipelined request failed: {}", err),
        //             }
        //         }
        //         Ok(())
        //     }).await;
        //     let _ignore_closed_channel = verify_result_tx.send(result);
        // });
        // old code for reference?
        tokio::spawn(
            ReceiverStream::new(verify_queue_rx)
                .map(Ok::<_, Error>)
                .try_for_each(move |response: h2::client::ResponseFuture| {
                    response
                        .map_err(Error::from)
                        .and_then(H2Client::h2api_response)
                        .map_ok(move |result| log::debug!("RESPONSE: {:?}", result))
                        .map_err(|err| format_err!("pipelined request failed: {}", err))
                })
                .map(|result| {
                    let _ignore_closed_channel = verify_result_tx.send(result);
                }),
        );

        (verify_queue_tx, verify_result_rx)
    }

    fn append_chunk_queue(
        h2: H2Client,
        wid: u64,
        path: String,
    ) -> (UploadQueueSender, UploadResultReceiver) {
        let (verify_queue_tx, verify_queue_rx) = mpsc::channel(64);
        let (verify_result_tx, verify_result_rx) = oneshot::channel();

        // FIXME: async-block-ify this code!
        tokio::spawn(
            ReceiverStream::new(verify_queue_rx)
                .map(Ok::<_, Error>)
                .and_then(move |(merged_chunk_info, response): (MergedChunkInfo, Option<h2::client::ResponseFuture>)| {
                    match (response, merged_chunk_info) {
                        (Some(response), MergedChunkInfo::Known(list)) => {
                            Either::Left(
                                response
                                    .map_err(Error::from)
                                    .and_then(H2Client::h2api_response)
                                    .and_then(move |_result| {
                                        future::ok(MergedChunkInfo::Known(list))
                                    })
                            )
                        }
                        (None, MergedChunkInfo::Known(list)) => {
                            Either::Right(future::ok(MergedChunkInfo::Known(list)))
                        }
                        _ => unreachable!(),
                    }
                })
                .merge_known_chunks()
                .and_then(move |merged_chunk_info| {
                    match merged_chunk_info {
                        MergedChunkInfo::Known(chunk_list) => {
                            let mut digest_list = vec![];
                            let mut offset_list = vec![];
                            for (offset, digest) in chunk_list {
                                digest_list.push(hex::encode(digest));
                                offset_list.push(offset);
                            }
                            log::debug!("append chunks list len ({})", digest_list.len());
                            let param = json!({ "wid": wid, "digest-list": digest_list, "offset-list": offset_list });
                            let request = H2Client::request_builder("localhost", "PUT", &path, None, Some("application/json")).unwrap();
                            let param_data = bytes::Bytes::from(param.to_string().into_bytes());
                            let upload_data = Some(param_data);
                            h2.send_request(request, upload_data)
                                .and_then(move |response| {
                                    response
                                        .map_err(Error::from)
                                        .and_then(H2Client::h2api_response)
                                        .map_ok(|_| ())
                                })
                                .map_err(|err| format_err!("pipelined request failed: {}", err))
                        }
                        _ => unreachable!(),
                    }
                })
                .try_for_each(|_| future::ok(()))
                .map(|result| {
                      let _ignore_closed_channel = verify_result_tx.send(result);
                })
        );

        (verify_queue_tx, verify_result_rx)
    }

    pub async fn download_previous_fixed_index(
        &self,
        archive_name: &str,
        manifest: &BackupManifest,
        known_chunks: Arc<Mutex<HashSet<[u8; 32]>>>,
    ) -> Result<FixedIndexReader, Error> {
        let mut tmpfile = std::fs::OpenOptions::new()
            .write(true)
            .read(true)
            .custom_flags(libc::O_TMPFILE)
            .open("/tmp")?;

        let param = json!({ "archive-name": archive_name });
        self.h2
            .download("previous", Some(param), &mut tmpfile)
            .await?;

        let index = FixedIndexReader::new(tmpfile).map_err(|err| {
            format_err!("unable to read fixed index '{}' - {}", archive_name, err)
        })?;
        // Note: do not use values stored in index (not trusted) - instead, computed them again
        let (csum, size) = index.compute_csum();
        manifest.verify_file(archive_name, &csum, size)?;

        // add index chunks to known chunks
        let mut known_chunks = known_chunks.lock().unwrap();
        for i in 0..index.index_count() {
            known_chunks.insert(*index.index_digest(i).unwrap());
        }

        log::debug!(
            "{}: known chunks list length is {}",
            archive_name,
            index.index_count()
        );

        Ok(index)
    }

    pub async fn download_previous_dynamic_index(
        &self,
        archive_name: &str,
        manifest: &BackupManifest,
        known_chunks: Arc<Mutex<HashSet<[u8; 32]>>>,
    ) -> Result<DynamicIndexReader, Error> {
        let mut tmpfile = std::fs::OpenOptions::new()
            .write(true)
            .read(true)
            .custom_flags(libc::O_TMPFILE)
            .open("/tmp")?;

        let param = json!({ "archive-name": archive_name });
        self.h2
            .download("previous", Some(param), &mut tmpfile)
            .await?;

        let index = DynamicIndexReader::new(tmpfile).map_err(|err| {
            format_err!("unable to read dynmamic index '{}' - {}", archive_name, err)
        })?;
        // Note: do not use values stored in index (not trusted) - instead, computed them again
        let (csum, size) = index.compute_csum();
        manifest.verify_file(archive_name, &csum, size)?;

        // add index chunks to known chunks
        let mut known_chunks = known_chunks.lock().unwrap();
        for i in 0..index.index_count() {
            known_chunks.insert(*index.index_digest(i).unwrap());
        }

        log::debug!(
            "{}: known chunks list length is {}",
            archive_name,
            index.index_count()
        );

        Ok(index)
    }

    /// Retrieve backup time of last backup
    pub async fn previous_backup_time(&self) -> Result<Option<i64>, Error> {
        let data = self.h2.get("previous_backup_time", None).await?;
        serde_json::from_value(data).map_err(|err| {
            format_err!(
                "Failed to parse backup time value returned by server - {}",
                err
            )
        })
    }

    /// Download backup manifest (index.json) of last backup
    pub async fn download_previous_manifest(&self) -> Result<BackupManifest, Error> {
        let mut raw_data = Vec::with_capacity(64 * 1024);

        let param = json!({ "archive-name": MANIFEST_BLOB_NAME });
        self.h2
            .download("previous", Some(param), &mut raw_data)
            .await?;

        let blob = DataBlob::load_from_reader(&mut &raw_data[..])?;
        // no expected digest available
        let data = blob.decode(self.crypt_config.as_ref().map(Arc::as_ref), None)?;

        let manifest =
            BackupManifest::from_data(&data[..], self.crypt_config.as_ref().map(Arc::as_ref))?;

        Ok(manifest)
    }

    // We have no `self` here for `h2` and `verbose`, the only other arg "common" with 1 other
    // function in the same path is `wid`, so those 3 could be in a struct, but there's no real use
    // since this is a private method.
    #[allow(clippy::too_many_arguments)]
    fn upload_chunk_info_stream(
        h2: H2Client,
        wid: u64,
        stream: impl Stream<Item = Result<bytes::BytesMut, Error>>,
        prefix: &str,
        known_chunks: Arc<Mutex<HashSet<[u8; 32]>>>,
        crypt_config: Option<Arc<CryptConfig>>,
        compress: bool,
    ) -> impl Future<Output = Result<UploadStats, Error>> {
        let total_chunks = Arc::new(AtomicUsize::new(0));
        let total_chunks2 = total_chunks.clone();
        let known_chunk_count = Arc::new(AtomicUsize::new(0));
        let known_chunk_count2 = known_chunk_count.clone();

        let stream_len = Arc::new(AtomicUsize::new(0));
        let stream_len2 = stream_len.clone();
        let compressed_stream_len = Arc::new(AtomicU64::new(0));
        let compressed_stream_len2 = compressed_stream_len.clone();
        let reused_len = Arc::new(AtomicUsize::new(0));
        let reused_len2 = reused_len.clone();

        let append_chunk_path = format!("{}_index", prefix);
        let upload_chunk_path = format!("{}_chunk", prefix);
        let is_fixed_chunk_size = prefix == "fixed";

        let (upload_queue, upload_result) =
            Self::append_chunk_queue(h2.clone(), wid, append_chunk_path);

        let start_time = std::time::Instant::now();

        let index_csum = Arc::new(Mutex::new(Some(openssl::sha::Sha256::new())));
        let index_csum_2 = index_csum.clone();

        stream
            .and_then(move |data| {
                let chunk_len = data.len();

                total_chunks.fetch_add(1, Ordering::SeqCst);
                let offset = stream_len.fetch_add(chunk_len, Ordering::SeqCst) as u64;

                let mut chunk_builder = DataChunkBuilder::new(data.as_ref()).compress(compress);

                if let Some(ref crypt_config) = crypt_config {
                    chunk_builder = chunk_builder.crypt_config(crypt_config);
                }

                let mut known_chunks = known_chunks.lock().unwrap();
                let digest = chunk_builder.digest();

                let mut guard = index_csum.lock().unwrap();
                let csum = guard.as_mut().unwrap();

                let chunk_end = offset + chunk_len as u64;

                if !is_fixed_chunk_size {
                    csum.update(&chunk_end.to_le_bytes());
                }
                csum.update(digest);

                let chunk_is_known = known_chunks.contains(digest);
                if chunk_is_known {
                    known_chunk_count.fetch_add(1, Ordering::SeqCst);
                    reused_len.fetch_add(chunk_len, Ordering::SeqCst);
                    future::ok(MergedChunkInfo::Known(vec![(offset, *digest)]))
                } else {
                    let compressed_stream_len2 = compressed_stream_len.clone();
                    known_chunks.insert(*digest);
                    future::ready(chunk_builder.build().map(move |(chunk, digest)| {
                        compressed_stream_len2.fetch_add(chunk.raw_size(), Ordering::SeqCst);
                        MergedChunkInfo::New(ChunkInfo {
                            chunk,
                            digest,
                            chunk_len: chunk_len as u64,
                            offset,
                        })
                    }))
                }
            })
            .merge_known_chunks()
            .try_for_each(move |merged_chunk_info| {
                let upload_queue = upload_queue.clone();

                if let MergedChunkInfo::New(chunk_info) = merged_chunk_info {
                    let offset = chunk_info.offset;
                    let digest = chunk_info.digest;
                    let digest_str = hex::encode(digest);

                    log::trace!(
                        "upload new chunk {} ({} bytes, offset {})",
                        digest_str,
                        chunk_info.chunk_len,
                        offset
                    );

                    let chunk_data = chunk_info.chunk.into_inner();
                    let param = json!({
                        "wid": wid,
                        "digest": digest_str,
                        "size": chunk_info.chunk_len,
                        "encoded-size": chunk_data.len(),
                    });

                    let ct = "application/octet-stream";
                    let request = H2Client::request_builder(
                        "localhost",
                        "POST",
                        &upload_chunk_path,
                        Some(param),
                        Some(ct),
                    )
                    .unwrap();
                    let upload_data = Some(bytes::Bytes::from(chunk_data));

                    let new_info = MergedChunkInfo::Known(vec![(offset, digest)]);

                    Either::Left(h2.send_request(request, upload_data).and_then(
                        move |response| async move {
                            upload_queue
                                .send((new_info, Some(response)))
                                .await
                                .map_err(|err| {
                                    format_err!("failed to send to upload queue: {}", err)
                                })
                        },
                    ))
                } else {
                    Either::Right(async move {
                        upload_queue
                            .send((merged_chunk_info, None))
                            .await
                            .map_err(|err| format_err!("failed to send to upload queue: {}", err))
                    })
                }
            })
            .then(move |result| async move { upload_result.await?.and(result) }.boxed())
            .and_then(move |_| {
                let duration = start_time.elapsed();
                let chunk_count = total_chunks2.load(Ordering::SeqCst);
                let chunk_reused = known_chunk_count2.load(Ordering::SeqCst);
                let size = stream_len2.load(Ordering::SeqCst);
                let size_reused = reused_len2.load(Ordering::SeqCst);
                let size_compressed = compressed_stream_len2.load(Ordering::SeqCst) as usize;

                let mut guard = index_csum_2.lock().unwrap();
                let csum = guard.take().unwrap().finish();

                futures::future::ok(UploadStats {
                    chunk_count,
                    chunk_reused,
                    size,
                    size_reused,
                    size_compressed,
                    duration,
                    csum,
                })
            })
    }

    /// Upload speed test - prints result to stderr
    pub async fn upload_speedtest(&self) -> Result<f64, Error> {
        let mut data = vec![];
        // generate pseudo random byte sequence
        for i in 0..1024 * 1024 {
            for j in 0..4 {
                let byte = ((i >> (j << 3)) & 0xff) as u8;
                data.push(byte);
            }
        }

        let item_len = data.len();

        let mut repeat = 0;

        let (upload_queue, upload_result) = Self::response_queue();

        let start_time = std::time::Instant::now();

        loop {
            repeat += 1;
            if start_time.elapsed().as_secs() >= 5 {
                break;
            }

            log::debug!("send test data ({} bytes)", data.len());
            let request =
                H2Client::request_builder("localhost", "POST", "speedtest", None, None).unwrap();
            let request_future = self
                .h2
                .send_request(request, Some(bytes::Bytes::from(data.clone())))
                .await?;

            upload_queue.send(request_future).await?;
        }

        drop(upload_queue); // close queue

        let _ = upload_result.await?;

        log::info!(
            "Uploaded {} chunks in {} seconds.",
            repeat,
            start_time.elapsed().as_secs()
        );
        let speed = ((item_len * (repeat as usize)) as f64) / start_time.elapsed().as_secs_f64();
        log::info!(
            "Time per request: {} microseconds.",
            (start_time.elapsed().as_micros()) / (repeat as u128)
        );

        Ok(speed)
    }
}
