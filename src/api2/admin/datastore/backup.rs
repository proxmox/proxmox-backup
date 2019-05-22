use failure::*;

use std::sync::Arc;

use futures::*;
use hyper::header::{HeaderValue, UPGRADE};
use hyper::{Body, Response, StatusCode};
use hyper::http::request::Parts;
use chrono::{Local, TimeZone};

use serde_json::{json, Value};

use crate::tools;
use crate::tools::wrapped_reader_stream::*;
use crate::api_schema::router::*;
use crate::api_schema::*;
use crate::server::WorkerTask;
use crate::backup::*;

mod environment;
use environment::*;

mod service;
use service::*;

mod upload_chunk;
use upload_chunk::*;


pub fn api_method_upgrade_backup() -> ApiAsyncMethod {
    ApiAsyncMethod::new(
        upgrade_to_backup_protocol,
        ObjectSchema::new("Upgraded to backup protocol.")
            .required("store", StringSchema::new("Datastore name."))
            .required("backup-type", StringSchema::new("Backup type.")
                      .format(Arc::new(ApiStringFormat::Enum(vec!["vm".into(), "ct".into(), "host".into()]))))
            .required("backup-id", StringSchema::new("Backup ID."))
    )
}

fn upgrade_to_backup_protocol(
    parts: Parts,
    req_body: Body,
    param: Value,
    _info: &ApiAsyncMethod,
    rpcenv: Box<RpcEnvironment>,
) -> Result<BoxFut, Error> {

    static PROXMOX_BACKUP_PROTOCOL_ID: &str = "proxmox-backup-protocol-h2";

    let store = tools::required_string_param(&param, "store")?.to_owned();
    let datastore = DataStore::lookup_datastore(&store)?;

    let backup_type = tools::required_string_param(&param, "backup-type")?;
    let backup_id = tools::required_string_param(&param, "backup-id")?;
    let backup_time = Local.timestamp(Local::now().timestamp(), 0);

    let protocols = parts
        .headers
        .get("UPGRADE")
        .ok_or_else(|| format_err!("missing Upgrade header"))?
        .to_str()?;

    if protocols != PROXMOX_BACKUP_PROTOCOL_ID {
        bail!("invalid protocol name");
    }

    if parts.version >=  http::version::Version::HTTP_2 {
        bail!("unexpected http version '{:?}' (expected version < 2)", parts.version);
    }

    let worker_id = format!("{}_{}_{}", store, backup_type, backup_id);

    let username = rpcenv.get_user().unwrap();
    let env_type = rpcenv.env_type();

    let backup_group = BackupGroup::new(backup_type, backup_id);
    let last_backup = BackupInfo::last_backup(&datastore.base_path(), &backup_group).unwrap_or(None);
    let backup_dir = BackupDir::new_with_group(backup_group, backup_time.timestamp());

    let (path, is_new) = datastore.create_backup_dir(&backup_dir)?;
    if !is_new { bail!("backup directorty already exists."); }

    WorkerTask::spawn("backup", Some(worker_id), &username.clone(), true, move |worker| {
        let mut env = BackupEnvironment::new(
            env_type, username.clone(), worker.clone(), datastore, backup_dir);

        env.last_backup = last_backup;

        env.log(format!("starting new backup on datastore '{}': {:?}", store, path));

        let service = BackupService::new(env.clone(), worker.clone());

        let abort_future = worker.abort_future();

        let env2 = env.clone();

        req_body
            .on_upgrade()
            .map_err(Error::from)
            .and_then(move |conn| {
                worker.log("upgrade done");

                let mut http = hyper::server::conn::Http::new();
                http.http2_only(true);
                // increase window size: todo - find optiomal size
                http.http2_initial_stream_window_size( (1 << 31) - 2);
                http.http2_initial_connection_window_size( (1 << 31) - 2);

                http.serve_connection(conn, service)
                    .map_err(Error::from)
             })
            .select(abort_future.map_err(|_| {}).then(move |_| { bail!("task aborted"); }))
            .map_err(|(err, _)| err)
            .and_then(move |(_result, _)| {
                env.ensure_finished()?;
                env.log("backup finished sucessfully");
                Ok(())
            })
            .then(move |result| {
                if let Err(err) = result {
                    match env2.ensure_finished() {
                        Ok(()) => {}, // ignorte error after finish
                        _ => {
                            env2.log(format!("backup failed: {}", err));
                            env2.log("removing failed backup");
                            env2.remove_backup()?;
                            return Err(err);
                        }
                    }
                }
                Ok(())
            })
    })?;

    let response = Response::builder()
        .status(StatusCode::SWITCHING_PROTOCOLS)
        .header(UPGRADE, HeaderValue::from_static(PROXMOX_BACKUP_PROTOCOL_ID))
        .body(Body::empty())?;

    Ok(Box::new(futures::future::ok(response)))
}

fn backup_api() -> Router {

    let router = Router::new()
        .subdir(
            "dynamic_chunk", Router::new()
                .upload(api_method_upload_dynamic_chunk())
        )
        .subdir(
            "dynamic_index", Router::new()
                .download(api_method_dynamic_chunk_index())
                .post(api_method_create_dynamic_index())
                .put(api_method_dynamic_append())
        )
        .subdir(
            "dynamic_close", Router::new()
                .post(api_method_close_dynamic_index())
        )
        .subdir(
            "finish", Router::new()
                .post(
                    ApiMethod::new(
                        finish_backup,
                        ObjectSchema::new("Mark backup as finished.")
                    )
                )
        )
        .subdir(
            "speedtest", Router::new()
                .upload(api_method_upload_speedtest())
        )
        .list_subdirs();

    router
}

pub fn api_method_dynamic_chunk_index() -> ApiAsyncMethod {
    ApiAsyncMethod::new(
        dynamic_chunk_index,
        ObjectSchema::new(r###"
Download the dynamic chunk index from the previous backup.
Simply returns an empty list if this is the first backup.
"###
        )
            .required("archive-name", crate::api2::types::BACKUP_ARCHIVE_NAME_SCHEMA.clone())
    )
}

pub fn api_method_create_dynamic_index() -> ApiMethod {
    ApiMethod::new(
        create_dynamic_index,
        ObjectSchema::new("Create dynamic chunk index file.")
            .required("archive-name", crate::api2::types::BACKUP_ARCHIVE_NAME_SCHEMA.clone())
    )
}

fn create_dynamic_index(
    param: Value,
    _info: &ApiMethod,
    rpcenv: &mut RpcEnvironment,
) -> Result<Value, Error> {

    let env: &BackupEnvironment = rpcenv.as_ref();

    let mut archive_name = tools::required_string_param(&param, "archive-name")?.to_owned();

    if !archive_name.ends_with(".pxar") {
        bail!("wrong archive extension");
    } else {
        archive_name.push_str(".didx");
    }

    let mut path = env.backup_dir.relative_path();
    path.push(archive_name);

    let chunk_size = 4096*1024; // todo: ??

    let index = env.datastore.create_dynamic_writer(&path, chunk_size)?;
    let wid = env.register_dynamic_writer(index)?;

    env.log(format!("created new dynamic index {} ({:?})", wid, path));

    Ok(json!(wid))
}

pub fn api_method_dynamic_append() -> ApiMethod {
    ApiMethod::new(
        dynamic_append,
        ObjectSchema::new("Append chunk to dynamic index writer.")
            .required("digest", StringSchema::new("Chunk digest."))
            .required("wid", IntegerSchema::new("Dynamic writer ID.")
                      .minimum(1)
                      .maximum(256)
            )
    )
}

fn dynamic_append (
    param: Value,
    _info: &ApiMethod,
    rpcenv: &mut RpcEnvironment,
) -> Result<Value, Error> {

    let wid = tools::required_integer_param(&param, "wid")? as usize;
    let digest_str = tools::required_string_param(&param, "digest")?;

    let env: &BackupEnvironment = rpcenv.as_ref();

    let digest = crate::tools::hex_to_digest(digest_str)?;
    let size = env.lookup_chunk(&digest).ok_or_else(|| format_err!("no such chunk {}", digest_str))?;

    env.dynamic_writer_append_chunk(wid, size, &digest)?;

    env.log(format!("sucessfully added chunk {} to dynamic index {}", digest_str, wid));

    Ok(Value::Null)
}

pub fn api_method_close_dynamic_index() -> ApiMethod {
    ApiMethod::new(
        close_dynamic_index,
        ObjectSchema::new("Close dynamic index writer.")
            .required("wid", IntegerSchema::new("Dynamic writer ID.")
                      .minimum(1)
                      .maximum(256)
            )
    )
}

fn close_dynamic_index (
    param: Value,
    _info: &ApiMethod,
    rpcenv: &mut RpcEnvironment,
) -> Result<Value, Error> {

    let wid = tools::required_integer_param(&param, "wid")? as usize;

    let env: &BackupEnvironment = rpcenv.as_ref();

    env.dynamic_writer_close(wid)?;

    env.log(format!("sucessfully closed dynamic index {}", wid));

    Ok(Value::Null)
}


fn finish_backup (
    _param: Value,
    _info: &ApiMethod,
    rpcenv: &mut RpcEnvironment,
) -> Result<Value, Error> {

    let env: &BackupEnvironment = rpcenv.as_ref();

    env.finish_backup()?;

    Ok(Value::Null)
}

fn dynamic_chunk_index(
    _parts: Parts,
    _req_body: Body,
    param: Value,
    _info: &ApiAsyncMethod,
    rpcenv: Box<RpcEnvironment>,
) -> Result<BoxFut, Error> {

    let env: &BackupEnvironment = rpcenv.as_ref();

    println!("TEST CHUNK DOWNLOAD");

    let mut archive_name = tools::required_string_param(&param, "archive-name")?.to_owned();

    if !archive_name.ends_with(".pxar") {
        bail!("wrong archive extension");
    } else {
        archive_name.push_str(".didx");
    }

    let empty_response = {
        Response::builder()
            .status(StatusCode::OK)
            .body(Body::empty())?
    };

    let last_backup = match &env.last_backup {
        Some(info) => info,
        None => return Ok(Box::new(future::ok(empty_response))),
    };

    let mut path = last_backup.backup_dir.relative_path();
    path.push(&archive_name);

    let index = match env.datastore.open_dynamic_reader(path) {
        Ok(index) => index,
        Err(_) => {
            env.log(format!("there is no last backup for archive '{}'", archive_name));
            return Ok(Box::new(future::ok(empty_response)));
        }
    };

    let count = index.index_count();
    for pos in 0..count {
        let (start, end, digest) = index.chunk_info(pos)?;
        let size = (end - start) as u32;
        env.register_chunk(digest, size)?;
    }

    let reader = DigestListEncoder::new(Box::new(index));

    let stream = WrappedReaderStream::new(reader);

    // fixme: set size, content type?
    let response = http::Response::builder()
        .status(200)
        .body(Body::wrap_stream(stream))?;

    Ok(Box::new(future::ok(response)))
}
