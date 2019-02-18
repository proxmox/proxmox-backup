use failure::*;

use crate::tools;
use crate::tools::wrapped_reader_stream::*;
use crate::backup::*;
//use crate::server::rest::*;
use crate::api_schema::*;
use crate::api_schema::router::*;

use chrono::{Utc, TimeZone};

use serde_json::Value;
use std::io::Write;
use futures::*;
use std::path::PathBuf;
use std::sync::Arc;

use hyper::Body;
use hyper::http::request::Parts;

pub struct UploadCaTar {
    stream: Body,
    index: DynamicIndexWriter,
    count: usize,
}

impl Future for UploadCaTar {
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

fn upload_catar(
    parts: Parts,
    req_body: Body,
    param: Value,
    _info: &ApiAsyncMethod,
    _rpcenv: &mut RpcEnvironment,
) -> Result<BoxFut, Error> {

    let store = tools::required_string_param(&param, "store")?;
    let mut archive_name = String::from(tools::required_string_param(&param, "archive_name")?);

    if !archive_name.ends_with(".catar") {
        bail!("got wront file extension (expected '.catar')");
    }

    archive_name.push_str(".didx");

    let backup_type = tools::required_string_param(&param, "type")?;
    let backup_id = tools::required_string_param(&param, "id")?;
    let backup_time = tools::required_integer_param(&param, "time")?;

    println!("Upload {}/{}/{}/{}/{}", store, backup_type, backup_id, backup_time, archive_name);

    let content_type = parts.headers.get(http::header::CONTENT_TYPE)
        .ok_or(format_err!("missing content-type header"))?;

    if content_type != "application/x-proxmox-backup-catar" {
        bail!("got wrong content-type for catar archive upload");
    }

    let chunk_size = 4*1024*1024;

    let datastore = DataStore::lookup_datastore(store)?;

    let mut path = datastore.create_backup_dir(backup_type, backup_id, backup_time)?;

    path.push(archive_name);

    let index = datastore.create_dynamic_writer(path, chunk_size)?;

    let upload = UploadCaTar { stream: req_body, index, count: 0};

    let resp = upload.and_then(|_| {

        let response = http::Response::builder()
            .status(200)
            .body(hyper::Body::empty())
            .unwrap();

        Ok(response)
    });

    Ok(Box::new(resp))
}

pub fn api_method_upload_catar() -> ApiAsyncMethod {
    ApiAsyncMethod::new(
        upload_catar,
        ObjectSchema::new("Upload .catar backup file.")
            .required("store", StringSchema::new("Datastore name."))
            .required("archive_name", StringSchema::new("Backup archive name."))
            .required("type", StringSchema::new("Backup type.")
                      .format(Arc::new(ApiStringFormat::Enum(vec!["ct".into(), "host".into()]))))
            .required("id", StringSchema::new("Backup ID."))
            .required("time", IntegerSchema::new("Backup time (Unix epoch.)")
                      .minimum(1547797308))

    )
}

fn download_catar(
    _parts: Parts,
    _req_body: Body,
    param: Value,
    _info: &ApiAsyncMethod,
    _rpcenv: &mut RpcEnvironment,
) -> Result<BoxFut, Error> {

    let store = tools::required_string_param(&param, "store")?;
    let archive_name = tools::required_string_param(&param, "archive_name")?;

    let backup_type = tools::required_string_param(&param, "type")?;
    let backup_id = tools::required_string_param(&param, "id")?;
    let backup_time = tools::required_integer_param(&param, "time")?;
    let backup_time = Utc.timestamp(backup_time, 0);

    println!("Download {}.catar from {} ({}/{}/{}/{}.didx)", archive_name, store,
             backup_type, backup_id, backup_time, archive_name);

    let datastore = DataStore::lookup_datastore(store)?;

    let mut path = datastore.get_backup_dir(backup_type, backup_id, backup_time);

    let mut full_archive_name = PathBuf::from(archive_name);
    full_archive_name.set_extension("didx");

    path.push(full_archive_name);

    let index = datastore.open_dynamic_reader(path)?;
    let reader = BufferedDynamicReader::new(index);
    let stream = WrappedReaderStream::new(reader);

    // fixme: set size, content type?
    let response = http::Response::builder()
        .status(200)
        .body(Body::wrap_stream(stream))?;

    Ok(Box::new(future::ok(response)))
}

pub fn api_method_download_catar() -> ApiAsyncMethod {
    ApiAsyncMethod::new(
        download_catar,
        ObjectSchema::new("Download .catar backup file.")
            .required("store", StringSchema::new("Datastore name."))
            .required("archive_name", StringSchema::new("Backup archive name."))
            .required("type", StringSchema::new("Backup type.")
                      .format(Arc::new(ApiStringFormat::Enum(vec!["ct".into(), "host".into()]))))
            .required("id", StringSchema::new("Backup ID."))
            .required("time", IntegerSchema::new("Backup time (Unix epoch.)")
                      .minimum(1547797308))

    )
}
