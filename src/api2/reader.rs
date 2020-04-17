//use chrono::{Local, TimeZone};
use anyhow::{bail, format_err, Error};
use futures::*;
use hyper::header::{self, HeaderValue, UPGRADE};
use hyper::http::request::Parts;
use hyper::{Body, Response, StatusCode};
use serde_json::Value;

use proxmox::{sortable, identity};
use proxmox::api::{ApiResponseFuture, ApiHandler, ApiMethod, Router, RpcEnvironment, Permission};
use proxmox::api::schema::*;
use proxmox::http_err;

use crate::api2::types::*;
use crate::backup::*;
use crate::server::{WorkerTask, H2Service};
use crate::tools;
use crate::config::acl::PRIV_DATASTORE_ALLOCATE_SPACE;

mod environment;
use environment::*;

pub const ROUTER: Router = Router::new()
    .upgrade(&API_METHOD_UPGRADE_BACKUP);

#[sortable]
pub const API_METHOD_UPGRADE_BACKUP: ApiMethod = ApiMethod::new(
    &ApiHandler::AsyncHttp(&upgrade_to_backup_reader_protocol),
    &ObjectSchema::new(
        concat!("Upgraded to backup protocol ('", PROXMOX_BACKUP_READER_PROTOCOL_ID_V1!(), "')."),
        &sorted!([
            ("store", false, &DATASTORE_SCHEMA),
            ("backup-type", false, &StringSchema::new("Backup type.")
             .format(&ApiStringFormat::Enum(&["vm", "ct", "host"]))
             .schema()
            ),
            ("backup-id", false, &StringSchema::new("Backup ID.").schema()),
            ("backup-time", false, &IntegerSchema::new("Backup time (Unix epoch.)")
             .minimum(1_547_797_308)
             .schema()
            ),
            ("debug", true, &BooleanSchema::new("Enable verbose debug logging.").schema()),
        ]),
    )
).access(None, &Permission::Privilege(&["datastore", "{store}"], PRIV_DATASTORE_ALLOCATE_SPACE, false));

fn upgrade_to_backup_reader_protocol(
    parts: Parts,
    req_body: Body,
    param: Value,
    _info: &ApiMethod,
    rpcenv: Box<dyn RpcEnvironment>,
) -> ApiResponseFuture {

    async move {
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

            let service = H2Service::new(env.clone(), worker.clone(), &READER_API_ROUTER, debug);

            let abort_future = worker.abort_future();

            let req_fut = req_body
                .on_upgrade()
                .map_err(Error::from)
                .and_then({
                    let env = env.clone();
                    move |conn| {
                        env.debug("protocol upgrade done");

                        let mut http = hyper::server::conn::Http::new();
                        http.http2_only(true);
                        // increase window size: todo - find optiomal size
                        let window_size = 32*1024*1024; // max = (1 << 31) - 2
                        http.http2_initial_stream_window_size(window_size);
                        http.http2_initial_connection_window_size(window_size);

                        http.serve_connection(conn, service)
                            .map_err(Error::from)
                    }
                });
            let abort_future = abort_future
                .map(|_| Err(format_err!("task aborted")));

            use futures::future::Either;
            futures::future::select(req_fut, abort_future)
                .map(|res| match res {
                    Either::Left((Ok(res), _)) => Ok(res),
                    Either::Left((Err(err), _)) => Err(err),
                    Either::Right((Ok(res), _)) => Ok(res),
                    Either::Right((Err(err), _)) => Err(err),
                })
                .map_ok(move |_| env.log("reader finished sucessfully"))
        })?;

        let response = Response::builder()
            .status(StatusCode::SWITCHING_PROTOCOLS)
            .header(UPGRADE, HeaderValue::from_static(PROXMOX_BACKUP_READER_PROTOCOL_ID_V1!()))
            .body(Body::empty())?;

        Ok(response)
    }.boxed()
}

pub const READER_API_ROUTER: Router = Router::new()
    .subdirs(&[
        (
            "chunk", &Router::new()
                .download(&API_METHOD_DOWNLOAD_CHUNK)
        ),
        (
            "download", &Router::new()
                .download(&API_METHOD_DOWNLOAD_FILE)
        ),
        (
            "speedtest", &Router::new()
                .download(&API_METHOD_SPEEDTEST)
        ),
    ]);

#[sortable]
pub const API_METHOD_DOWNLOAD_FILE: ApiMethod = ApiMethod::new(
    &ApiHandler::AsyncHttp(&download_file),
    &ObjectSchema::new(
        "Download specified file.",
        &sorted!([
            ("file-name", false, &crate::api2::types::BACKUP_ARCHIVE_NAME_SCHEMA),
        ]),
    )
);

fn download_file(
    _parts: Parts,
    _req_body: Body,
    param: Value,
    _info: &ApiMethod,
    rpcenv: Box<dyn RpcEnvironment>,
) -> ApiResponseFuture {

    async move {
        let env: &ReaderEnvironment = rpcenv.as_ref();

        let file_name = tools::required_string_param(&param, "file-name")?.to_owned();

        let mut path = env.datastore.base_path();
        path.push(env.backup_dir.relative_path());
        path.push(&file_name);

        let path2 = path.clone();
        let path3 = path.clone();

        let file = tokio::fs::File::open(path)
            .map_err(move |err| http_err!(BAD_REQUEST, format!("open file {:?} failed: {}", path2, err)))
            .await?;

        env.log(format!("download {:?}", path3));

        let payload = tokio_util::codec::FramedRead::new(file, tokio_util::codec::BytesCodec::new())
            .map_ok(|bytes| hyper::body::Bytes::from(bytes.freeze()));

        let body = Body::wrap_stream(payload);

        // fixme: set other headers ?
        Ok(Response::builder()
           .status(StatusCode::OK)
           .header(header::CONTENT_TYPE, "application/octet-stream")
           .body(body)
           .unwrap())
    }.boxed()
}

#[sortable]
pub const API_METHOD_DOWNLOAD_CHUNK: ApiMethod = ApiMethod::new(
    &ApiHandler::AsyncHttp(&download_chunk),
    &ObjectSchema::new(
        "Download specified chunk.",
        &sorted!([
            ("digest", false, &CHUNK_DIGEST_SCHEMA),
        ]),
    )
);

fn download_chunk(
    _parts: Parts,
    _req_body: Body,
    param: Value,
    _info: &ApiMethod,
    rpcenv: Box<dyn RpcEnvironment>,
) -> ApiResponseFuture {

    async move {
        let env: &ReaderEnvironment = rpcenv.as_ref();

        let digest_str = tools::required_string_param(&param, "digest")?;
        let digest = proxmox::tools::hex_to_digest(digest_str)?;

        let (path, _) = env.datastore.chunk_path(&digest);
        let path2 = path.clone();

        env.debug(format!("download chunk {:?}", path));

        let data = tokio::fs::read(path)
            .map_err(move |err| http_err!(BAD_REQUEST, format!("reading file {:?} failed: {}", path2, err)))
            .await?;

        let body = Body::from(data);

        // fixme: set other headers ?
        Ok(Response::builder()
           .status(StatusCode::OK)
           .header(header::CONTENT_TYPE, "application/octet-stream")
           .body(body)
           .unwrap())
    }.boxed()
}

/* this is too slow
fn download_chunk_old(
    _parts: Parts,
    _req_body: Body,
    param: Value,
    _info: &ApiMethod,
    rpcenv: Box<dyn RpcEnvironment>,
) -> Result<ApiResponseFuture, Error> {

    let env: &ReaderEnvironment = rpcenv.as_ref();
    let env2 = env.clone();

    let digest_str = tools::required_string_param(&param, "digest")?;
    let digest = proxmox::tools::hex_to_digest(digest_str)?;

    let (path, _) = env.datastore.chunk_path(&digest);

    let path2 = path.clone();
    let path3 = path.clone();

    let response_future = tokio::fs::File::open(path)
        .map_err(move |err| http_err!(BAD_REQUEST, format!("open file {:?} failed: {}", path2, err)))
        .and_then(move |file| {
            env2.debug(format!("download chunk {:?}", path3));
            let payload = tokio_util::codec::FramedRead::new(file, tokio_util::codec::BytesCodec::new())
                .map_ok(|bytes| hyper::body::Bytes::from(bytes.freeze()));

            let body = Body::wrap_stream(payload);

            // fixme: set other headers ?
            futures::future::ok(Response::builder()
               .status(StatusCode::OK)
               .header(header::CONTENT_TYPE, "application/octet-stream")
               .body(body)
               .unwrap())
        });

    Ok(Box::new(response_future))
}
*/

pub const API_METHOD_SPEEDTEST: ApiMethod = ApiMethod::new(
    &ApiHandler::AsyncHttp(&speedtest),
    &ObjectSchema::new("Test 4M block download speed.", &[])
);

fn speedtest(
    _parts: Parts,
    _req_body: Body,
    _param: Value,
    _info: &ApiMethod,
    _rpcenv: Box<dyn RpcEnvironment>,
) -> ApiResponseFuture {

    let buffer = vec![65u8; 1024*1024]; // nonsense [A,A,A...]

    let body = Body::from(buffer);

    let response = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(body)
        .unwrap();

    future::ok(response).boxed()
}
