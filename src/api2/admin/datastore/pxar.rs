use failure::*;

use crate::tools;
use crate::tools::wrapped_reader_stream::*;
use crate::backup::*;
use crate::api_schema::*;
use crate::api_schema::router::*;

use chrono::{Local, TimeZone};

use serde_json::Value;
use futures::*;
use std::sync::Arc;

use hyper::Body;
use hyper::http::request::Parts;

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
