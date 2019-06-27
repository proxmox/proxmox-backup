use failure::*;
use lazy_static::lazy_static;

use std::sync::Arc;

use futures::*;
use hyper::header::{self, HeaderValue, UPGRADE};
use hyper::{Body, Response, StatusCode};
use hyper::http::request::Parts;
//use chrono::{Local, TimeZone};

use serde_json::Value;

use crate::tools;
use crate::api_schema::router::*;
use crate::api_schema::*;
use crate::server::{WorkerTask, H2Service};
use crate::backup::*;
//use crate::api2::types::*;

mod environment;
use environment::*;

pub fn router() -> Router {
    Router::new()
        .upgrade(api_method_upgrade_backup())
}

pub fn api_method_upgrade_backup() -> ApiAsyncMethod {
    ApiAsyncMethod::new(
        upgrade_to_backup_reader_protocol,
        ObjectSchema::new(concat!("Upgraded to backup protocol ('", PROXMOX_BACKUP_READER_PROTOCOL_ID_V1!(), "')."))
            .required("store", StringSchema::new("Datastore name."))
            .required("backup-type", StringSchema::new("Backup type.")
                      .format(Arc::new(ApiStringFormat::Enum(&["vm", "ct", "host"]))))
            .required("backup-id", StringSchema::new("Backup ID."))
            .required("backup-time", IntegerSchema::new("Backup time (Unix epoch.)")
                      .minimum(1547797308))
            .optional("debug", BooleanSchema::new("Enable verbose debug logging."))
    )
}

fn upgrade_to_backup_reader_protocol(
    parts: Parts,
    req_body: Body,
    param: Value,
    _info: &ApiAsyncMethod,
    rpcenv: Box<dyn RpcEnvironment>,
) -> Result<BoxFut, Error> {

    let debug = param["debug"].as_bool().unwrap_or(false);

    let store = tools::required_string_param(&param, "store")?.to_owned();
    let datastore = DataStore::lookup_datastore(&store)?;

    let backup_type = tools::required_string_param(&param, "backup-type")?;
    let backup_id = tools::required_string_param(&param, "backup-id")?;
    let backup_time = tools::required_integer_param(&param, "backup-time")?;

    let protocols = parts
        .headers
        .get("UPGRADE")
        .ok_or_else(|| format_err!("missing Upgrade header"))?
        .to_str()?;

    if protocols != PROXMOX_BACKUP_READER_PROTOCOL_ID_V1!() {
        bail!("invalid protocol name");
    }

    if parts.version >=  http::version::Version::HTTP_2 {
        bail!("unexpected http version '{:?}' (expected version < 2)", parts.version);
    }

    let username = rpcenv.get_user().unwrap();
    let env_type = rpcenv.env_type();

    let backup_dir = BackupDir::new(backup_type, backup_id, backup_time);
    let path = datastore.base_path();

    //let files = BackupInfo::list_files(&path, &backup_dir)?;

    let worker_id = format!("{}_{}_{}_{:08X}", store, backup_type, backup_id, backup_dir.backup_time().timestamp());

    WorkerTask::spawn("reader", Some(worker_id), &username.clone(), true, move |worker| {
        let mut env = ReaderEnvironment::new(
            env_type, username.clone(), worker.clone(), datastore, backup_dir);

        env.debug = debug;

        env.log(format!("starting new backup reader datastore '{}': {:?}", store, path));

        let service = H2Service::new(env.clone(), worker.clone(), &READER_ROUTER, debug);

        let abort_future = worker.abort_future();

        let env3 = env.clone();

        req_body
            .on_upgrade()
            .map_err(Error::from)
            .and_then(move |conn| {
                env3.debug("protocol upgrade done");

                let mut http = hyper::server::conn::Http::new();
                http.http2_only(true);
                // increase window size: todo - find optiomal size
                let window_size = 32*1024*1024; // max = (1 << 31) - 2
                http.http2_initial_stream_window_size(window_size);
                http.http2_initial_connection_window_size(window_size);

                http.serve_connection(conn, service)
                    .map_err(Error::from)
             })
            .select(abort_future.map_err(|_| {}).then(move |_| { bail!("task aborted"); }))
            .map_err(|(err, _)| err)
            .and_then(move |(_result, _)| {
                env.log("reader finished sucessfully");
                Ok(())
            })
    })?;

    let response = Response::builder()
        .status(StatusCode::SWITCHING_PROTOCOLS)
        .header(UPGRADE, HeaderValue::from_static(PROXMOX_BACKUP_READER_PROTOCOL_ID_V1!()))
        .body(Body::empty())?;

    Ok(Box::new(futures::future::ok(response)))
}

lazy_static!{
    static ref READER_ROUTER: Router = reader_api();
}

pub fn reader_api() -> Router {

    let router = Router::new()
        .subdir(
            "download", Router::new()
                .download(api_method_download_file())
        );

    router
}

pub fn api_method_download_file() -> ApiAsyncMethod {
    ApiAsyncMethod::new(
        download_file,
        ObjectSchema::new("Download specified file.")
            .required("file-name", crate::api2::types::BACKUP_ARCHIVE_NAME_SCHEMA.clone())
    )
}

fn download_file(
    _parts: Parts,
    _req_body: Body,
    param: Value,
    _info: &ApiAsyncMethod,
    rpcenv: Box<dyn RpcEnvironment>,
) -> Result<BoxFut, Error> {

    let env: &ReaderEnvironment = rpcenv.as_ref();
    let env2 = env.clone();

    let file_name = tools::required_string_param(&param, "file-name")?.to_owned();

    let mut path = env.datastore.base_path();
    path.push(env.backup_dir.relative_path());
    path.push(&file_name);

    let path2 = path.clone();
    let path3 = path.clone();

    let response_future = tokio::fs::File::open(path)
        .map_err(move |err| http_err!(BAD_REQUEST, format!("open file {:?} failed: {}", path2, err)))
        .and_then(move |file| {
            env2.log(format!("download {:?}", path3));
            let payload = tokio::codec::FramedRead::new(file, tokio::codec::BytesCodec::new()).
                map(|bytes| {
                    //sigh - howto avoid copy here? or the whole map() ??
                    hyper::Chunk::from(bytes.to_vec())
                });
            let body = Body::wrap_stream(payload);

            // fixme: set other headers ?
            Ok(Response::builder()
               .status(StatusCode::OK)
               .header(header::CONTENT_TYPE, "application/octet-stream")
               .body(body)
               .unwrap())
        });

    Ok(Box::new(response_future))
}
