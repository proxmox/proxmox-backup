use failure::*;

use crate::tools;
use crate::tools::wrapped_reader_stream::*;
use crate::backup::*;
use crate::server;
use crate::api_schema::*;
use crate::api_schema::router::*;

use chrono::{Local, TimeZone};

use serde_json::Value;
use std::io::Write;
use futures::*;
//use std::path::PathBuf;
use std::sync::Arc;

use hyper::Body;
use hyper::http::request::Parts;

pub struct UploadPxar {
    stream: Body,
    index: DynamicChunkWriter,
    count: usize,
}

impl Future for UploadPxar {
    type Item = ();
    type Error = failure::Error;

    fn poll(&mut self) -> Poll<(), failure::Error> {
        loop {
            match try_ready!(self.stream.poll()) {
                Some(chunk) => {
                    self.count += chunk.len();
                    if let Err(err) = self.index.write_all(&chunk) {
                        bail!("writing chunk failed - {}", err);
                    }
                }
                None => {
                    self.index.close()?;
                    return Ok(Async::Ready(()))
                }
            }
        }
    }
}

fn upload_pxar(
    parts: Parts,
    req_body: Body,
    param: Value,
    _info: &ApiAsyncMethod,
    rpcenv: Box<dyn RpcEnvironment>,
) -> Result<BoxFut, Error> {

    let store = tools::required_string_param(&param, "store")?;
    let mut archive_name = String::from(tools::required_string_param(&param, "archive-name")?);

    if !archive_name.ends_with(".pxar") {
        bail!("got wront file extension (expected '.pxar')");
    }

    archive_name.push_str(".didx");

    let backup_type = tools::required_string_param(&param, "backup-type")?;
    let backup_id = tools::required_string_param(&param, "backup-id")?;
    let backup_time = tools::required_integer_param(&param, "backup-time")?;

    let worker_id = format!("{}_{}_{}_{}_{}", store, backup_type, backup_id, backup_time, archive_name);

    println!("Upload {}", worker_id);

    let content_type = parts.headers.get(http::header::CONTENT_TYPE)
        .ok_or(format_err!("missing content-type header"))?;

    if content_type != "application/x-proxmox-backup-pxar" {
        bail!("got wrong content-type for pxar archive upload");
    }

    let chunk_size = param["chunk-size"].as_u64().unwrap_or(4096*1024) as usize;
    verify_chunk_size(chunk_size)?;

    let datastore = DataStore::lookup_datastore(store)?;
    let backup_dir = BackupDir::new(backup_type, backup_id, backup_time);

    let (mut path, _new) = datastore.create_backup_dir(&backup_dir)?;

    path.push(archive_name);

    let index = datastore.create_dynamic_writer(path)?;
    let index = DynamicChunkWriter::new(index, chunk_size as usize);

    let upload = UploadPxar { stream: req_body, index, count: 0};

    let worker = server::WorkerTask::new("upload", Some(worker_id), &rpcenv.get_user().unwrap(), false)?;
    let worker1 = worker.clone();
    let abort_future = worker.abort_future();

    let resp = upload
        .select(abort_future.map_err(|_| {})
                .then(move |_| {
                    worker1.log("aborting task...");
                    bail!("task aborted");
                })
        )
        .then(move |result| {
            match result {
                Ok((result,_)) => worker.log_result(Ok(result)),
                Err((err, _)) =>  worker.log_result(Err(err)),
            }
            Ok(())
        })
        .and_then(|_| {

        let response = http::Response::builder()
            .status(200)
            .body(hyper::Body::empty())
            .unwrap();

        Ok(response)
    });

    Ok(Box::new(resp))
}

pub fn api_method_upload_pxar() -> ApiAsyncMethod {
    ApiAsyncMethod::new(
        upload_pxar,
        ObjectSchema::new("Upload .pxar backup file.")
            .required("store", StringSchema::new("Datastore name."))
            .required("archive-name", StringSchema::new("Backup archive name."))
            .required("backup-type", StringSchema::new("Backup type.")
                      .format(Arc::new(ApiStringFormat::Enum(&["ct", "host"]))))
            .required("backup-id", StringSchema::new("Backup ID."))
            .required("backup-time", IntegerSchema::new("Backup time (Unix epoch.)")
                      .minimum(1547797308))
            .optional(
                "chunk-size",
                IntegerSchema::new("Chunk size in bytes. Must be a power of 2.")
                    .minimum(64*1024)
                    .maximum(4096*1024)
                    .default(4096*1024)
            )
    )
}

fn download_pxar(
    _parts: Parts,
    _req_body: Body,
    param: Value,
    _info: &ApiAsyncMethod,
    _rpcenv: Box<dyn RpcEnvironment>,
) -> Result<BoxFut, Error> {

    let store = tools::required_string_param(&param, "store")?;
    let mut archive_name = tools::required_string_param(&param, "archive-name")?.to_owned();

    if !archive_name.ends_with(".pxar") {
        bail!("wrong archive extension");
    } else {
        archive_name.push_str(".didx");
    }

    let backup_type = tools::required_string_param(&param, "backup-type")?;
    let backup_id = tools::required_string_param(&param, "backup-id")?;
    let backup_time = tools::required_integer_param(&param, "backup-time")?;

    println!("Download {} from {} ({}/{}/{}/{})", archive_name, store,
             backup_type, backup_id, Local.timestamp(backup_time, 0), archive_name);

    let datastore = DataStore::lookup_datastore(store)?;

    let backup_dir = BackupDir::new(backup_type, backup_id, backup_time);

    let mut path = backup_dir.relative_path();
    path.push(archive_name);

    let index = datastore.open_dynamic_reader(path)?;
    let reader = BufferedDynamicReader::new(index);
    let stream = WrappedReaderStream::new(reader);

    // fixme: set size, content type?
    let response = http::Response::builder()
        .status(200)
        .body(Body::wrap_stream(stream))?;

    Ok(Box::new(future::ok(response)))
}

pub fn api_method_download_pxar() -> ApiAsyncMethod {
    ApiAsyncMethod::new(
        download_pxar,
        ObjectSchema::new("Download .pxar backup file.")
            .required("store", StringSchema::new("Datastore name."))
            .required("archive-name", StringSchema::new("Backup archive name."))
            .required("backup-type", StringSchema::new("Backup type.")
                      .format(Arc::new(ApiStringFormat::Enum(&["ct", "host"]))))
            .required("backup-id", StringSchema::new("Backup ID."))
            .required("backup-time", IntegerSchema::new("Backup time (Unix epoch.)")
                      .minimum(1547797308))

    )
}
