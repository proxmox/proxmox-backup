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
use crate::api2::types::*;

mod environment;
use environment::*;

mod service;
use service::*;

mod upload_chunk;
use upload_chunk::*;

pub fn router() -> Router {
    Router::new()
        .upgrade(api_method_upgrade_backup())
}

pub fn api_method_upgrade_backup() -> ApiAsyncMethod {
    ApiAsyncMethod::new(
        upgrade_to_backup_protocol,
        ObjectSchema::new(concat!("Upgraded to backup protocol ('", PROXMOX_BACKUP_PROTOCOL_ID_V1!(), "')."))
            .required("store", StringSchema::new("Datastore name."))
            .required("backup-type", StringSchema::new("Backup type.")
                      .format(Arc::new(ApiStringFormat::Enum(&["vm", "ct", "host"]))))
            .required("backup-id", StringSchema::new("Backup ID."))
            .optional("debug", BooleanSchema::new("Enable verbose debug logging."))
    )
}

fn upgrade_to_backup_protocol(
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
    let backup_time = Local.timestamp(Local::now().timestamp(), 0);

    let protocols = parts
        .headers
        .get("UPGRADE")
        .ok_or_else(|| format_err!("missing Upgrade header"))?
        .to_str()?;

    if protocols != PROXMOX_BACKUP_PROTOCOL_ID_V1!() {
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

        env.debug = debug;
        env.last_backup = last_backup;

        env.log(format!("starting new backup on datastore '{}': {:?}", store, path));

        let service = BackupService::new(env.clone(), worker.clone(), debug);

        let abort_future = worker.abort_future();

        let env2 = env.clone();
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
                env.ensure_finished()?;
                env.log("backup finished sucessfully");
                Ok(())
            })
            .then(move |result| {
                if let Err(err) = result {
                    match env2.ensure_finished() {
                        Ok(()) => {}, // ignore error after finish
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
        .header(UPGRADE, HeaderValue::from_static(PROXMOX_BACKUP_PROTOCOL_ID_V1!()))
        .body(Body::empty())?;

    Ok(Box::new(futures::future::ok(response)))
}

pub fn backup_api() -> Router {

    let router = Router::new()
        .subdir(
            "config", Router::new()
                .upload(api_method_upload_config())
        )
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
            "fixed_chunk", Router::new()
                .upload(api_method_upload_fixed_chunk())
        )
        .subdir(
            "fixed_index", Router::new()
                .download(api_method_fixed_chunk_index())
                .post(api_method_create_fixed_index())
                .put(api_method_fixed_append())
        )
        .subdir(
            "fixed_close", Router::new()
                .post(api_method_close_fixed_index())
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
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let env: &BackupEnvironment = rpcenv.as_ref();

    let name = tools::required_string_param(&param, "archive-name")?.to_owned();

    let mut archive_name = name.clone();
    if !archive_name.ends_with(".pxar") {
        bail!("wrong archive extension: '{}'", archive_name);
    } else {
        archive_name.push_str(".didx");
    }

    let mut path = env.backup_dir.relative_path();
    path.push(archive_name);

    let index = env.datastore.create_dynamic_writer(&path)?;
    let wid = env.register_dynamic_writer(index, name)?;

    env.log(format!("created new dynamic index {} ({:?})", wid, path));

    Ok(json!(wid))
}

pub fn api_method_create_fixed_index() -> ApiMethod {
    ApiMethod::new(
        create_fixed_index,
        ObjectSchema::new("Create fixed chunk index file.")
            .required("archive-name", crate::api2::types::BACKUP_ARCHIVE_NAME_SCHEMA.clone())
            .required("size", IntegerSchema::new("File size.")
                      .minimum(1)
            )
    )
}

fn create_fixed_index(
    param: Value,
    _info: &ApiMethod,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let env: &BackupEnvironment = rpcenv.as_ref();

    println!("PARAM: {:?}", param);

    let name = tools::required_string_param(&param, "archive-name")?.to_owned();
    let size = tools::required_integer_param(&param, "size")? as usize;

    let mut archive_name = name.clone();
    if !archive_name.ends_with(".img") {
        bail!("wrong archive extension: '{}'", archive_name);
    } else {
        archive_name.push_str(".fidx");
    }

    let mut path = env.backup_dir.relative_path();
    path.push(archive_name);

    let chunk_size = 4096*1024; // todo: ??

    let index = env.datastore.create_fixed_writer(&path, size, chunk_size)?;
    let wid = env.register_fixed_writer(index, name, size, chunk_size as u32)?;

    env.log(format!("created new fixed index {} ({:?})", wid, path));

    Ok(json!(wid))
}

pub fn api_method_dynamic_append() -> ApiMethod {
    ApiMethod::new(
        dynamic_append,
        ObjectSchema::new("Append chunk to dynamic index writer.")
            .required("wid", IntegerSchema::new("Dynamic writer ID.")
                      .minimum(1)
                      .maximum(256)
            )
            .required("digest-list", ArraySchema::new(
                "Chunk digest list.", CHUNK_DIGEST_SCHEMA.clone())
            )
            .required("offset-list", ArraySchema::new(
                "Chunk offset list.",
                IntegerSchema::new("Corresponding chunk offsets.")
                    .minimum(0)
                    .into())
            )
    )
}

fn dynamic_append (
    param: Value,
    _info: &ApiMethod,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let wid = tools::required_integer_param(&param, "wid")? as usize;
    let digest_list = tools::required_array_param(&param, "digest-list")?;
    let offset_list = tools::required_array_param(&param, "offset-list")?;

    if offset_list.len() != digest_list.len() {
        bail!("offset list has wrong length ({} != {})", offset_list.len(), digest_list.len());
    }

    let env: &BackupEnvironment = rpcenv.as_ref();

    env.debug(format!("dynamic_append {} chunks", digest_list.len()));

    for (i, item) in digest_list.iter().enumerate() {
        let digest_str = item.as_str().unwrap();
        let digest = proxmox::tools::hex_to_digest(digest_str)?;
        let offset = offset_list[i].as_u64().unwrap();
        let size = env.lookup_chunk(&digest).ok_or_else(|| format_err!("no such chunk {}", digest_str))?;

        env.dynamic_writer_append_chunk(wid, offset, size, &digest)?;

        env.debug(format!("sucessfully added chunk {} to dynamic index {} (offset {}, size {})", digest_str, wid, offset, size));
    }

    Ok(Value::Null)
}

pub fn api_method_fixed_append() -> ApiMethod {
    ApiMethod::new(
        fixed_append,
        ObjectSchema::new("Append chunk to fixed index writer.")
            .required("wid", IntegerSchema::new("Fixed writer ID.")
                      .minimum(1)
                      .maximum(256)
            )
            .required("digest-list", ArraySchema::new(
                "Chunk digest list.", CHUNK_DIGEST_SCHEMA.clone())
            )
            .required("offset-list", ArraySchema::new(
                "Chunk offset list.",
                IntegerSchema::new("Corresponding chunk offsets.")
                    .minimum(0)
                    .into())
            )
    )
}

fn fixed_append (
    param: Value,
    _info: &ApiMethod,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let wid = tools::required_integer_param(&param, "wid")? as usize;
    let digest_list = tools::required_array_param(&param, "digest-list")?;
    let offset_list = tools::required_array_param(&param, "offset-list")?;

    if offset_list.len() != digest_list.len() {
        bail!("offset list has wrong length ({} != {})", offset_list.len(), digest_list.len());
    }

    let env: &BackupEnvironment = rpcenv.as_ref();

    env.debug(format!("fixed_append {} chunks", digest_list.len()));

    for (i, item) in digest_list.iter().enumerate() {
        let digest_str = item.as_str().unwrap();
        let digest = proxmox::tools::hex_to_digest(digest_str)?;
        let offset = offset_list[i].as_u64().unwrap();
        let size = env.lookup_chunk(&digest).ok_or_else(|| format_err!("no such chunk {}", digest_str))?;

        env.fixed_writer_append_chunk(wid, offset, size, &digest)?;

        env.debug(format!("sucessfully added chunk {} to fixed index {} (offset {}, size {})", digest_str, wid, offset, size));
    }

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
            .required("chunk-count", IntegerSchema::new("Chunk count. This is used to verify that the server got all chunks.")
                      .minimum(1)
            )
            .required("size", IntegerSchema::new("File size. This is used to verify that the server got all data.")
                      .minimum(1)
            )
    )
}

fn close_dynamic_index (
    param: Value,
    _info: &ApiMethod,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let wid = tools::required_integer_param(&param, "wid")? as usize;
    let chunk_count = tools::required_integer_param(&param, "chunk-count")? as u64;
    let size = tools::required_integer_param(&param, "size")? as u64;

    let env: &BackupEnvironment = rpcenv.as_ref();

    env.dynamic_writer_close(wid, chunk_count, size)?;

    env.log(format!("sucessfully closed dynamic index {}", wid));

    Ok(Value::Null)
}

pub fn api_method_close_fixed_index() -> ApiMethod {
    ApiMethod::new(
        close_fixed_index,
        ObjectSchema::new("Close fixed index writer.")
            .required("wid", IntegerSchema::new("Fixed writer ID.")
                      .minimum(1)
                      .maximum(256)
            )
            .required("chunk-count", IntegerSchema::new("Chunk count. This is used to verify that the server got all chunks.")
                      .minimum(1)
            )
            .required("size", IntegerSchema::new("File size. This is used to verify that the server got all data.")
                      .minimum(1)
            )
    )
}

fn close_fixed_index (
    param: Value,
    _info: &ApiMethod,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let wid = tools::required_integer_param(&param, "wid")? as usize;
    let chunk_count = tools::required_integer_param(&param, "chunk-count")? as u64;
    let size = tools::required_integer_param(&param, "size")? as u64;

    let env: &BackupEnvironment = rpcenv.as_ref();

    env.fixed_writer_close(wid, chunk_count, size)?;

    env.log(format!("sucessfully closed fixed index {}", wid));

    Ok(Value::Null)
}

fn finish_backup (
    _param: Value,
    _info: &ApiMethod,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let env: &BackupEnvironment = rpcenv.as_ref();

    env.finish_backup()?;
    env.log("sucessfully finished backup");

    Ok(Value::Null)
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

fn dynamic_chunk_index(
    _parts: Parts,
    _req_body: Body,
    param: Value,
    _info: &ApiAsyncMethod,
    rpcenv: Box<dyn RpcEnvironment>,
) -> Result<BoxFut, Error> {

    let env: &BackupEnvironment = rpcenv.as_ref();

    let mut archive_name = tools::required_string_param(&param, "archive-name")?.to_owned();

    if !archive_name.ends_with(".pxar") {
        bail!("wrong archive extension: '{}'", archive_name);
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

    env.log(format!("download last backup index for archive '{}'", archive_name));

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

pub fn api_method_fixed_chunk_index() -> ApiAsyncMethod {
    ApiAsyncMethod::new(
        fixed_chunk_index,
        ObjectSchema::new(r###"
Download the fixed chunk index from the previous backup.
Simply returns an empty list if this is the first backup.
"###
        )
            .required("archive-name", crate::api2::types::BACKUP_ARCHIVE_NAME_SCHEMA.clone())
    )
}

fn fixed_chunk_index(
    _parts: Parts,
    _req_body: Body,
    param: Value,
    _info: &ApiAsyncMethod,
    rpcenv: Box<dyn RpcEnvironment>,
) -> Result<BoxFut, Error> {

    let env: &BackupEnvironment = rpcenv.as_ref();

    let mut archive_name = tools::required_string_param(&param, "archive-name")?.to_owned();

    if !archive_name.ends_with(".img") {
        bail!("wrong archive extension: '{}'", archive_name);
    } else {
        archive_name.push_str(".fidx");
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

    let index = match env.datastore.open_fixed_reader(path) {
        Ok(index) => index,
        Err(_) => {
            env.log(format!("there is no last backup for archive '{}'", archive_name));
            return Ok(Box::new(future::ok(empty_response)));
        }
    };

    env.log(format!("download last backup index for archive '{}'", archive_name));

    let count = index.index_count();
    for pos in 0..count {
        let digest = index.index_digest(pos).unwrap();
        let size = index.chunk_size as u32;
        env.register_chunk(*digest, size)?;
    }

    let reader = DigestListEncoder::new(Box::new(index));

    let stream = WrappedReaderStream::new(reader);

    // fixme: set size, content type?
    let response = http::Response::builder()
        .status(200)
        .body(Body::wrap_stream(stream))?;

    Ok(Box::new(future::ok(response)))
}
