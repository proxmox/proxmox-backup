use failure::*;

use std::sync::Arc;

use futures::*;
use hyper::header::{HeaderValue, UPGRADE};
use hyper::{Body, Response, StatusCode};
use hyper::http::request::Parts;
use chrono::{Local, TimeZone};

use serde_json::Value;

use crate::tools;
use crate::api_schema::router::*;
use crate::api_schema::*;
use crate::server::WorkerTask;

mod environment;
use environment::*;

mod service;
use service::*;

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
    rpcenv: &mut RpcEnvironment,
) -> Result<BoxFut, Error> {

    static PROXMOX_BACKUP_PROTOCOL_ID: &str = "proxmox-backup-protocol-h2";

    let store = tools::required_string_param(&param, "store")?;
    let backup_type = tools::required_string_param(&param, "backup-type")?;
    let backup_id = tools::required_string_param(&param, "backup-id")?;

    let _backup_time = Local.timestamp(Local::now().timestamp(), 0);

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

    WorkerTask::spawn("backup", Some(worker_id), &username.clone(), true, move |worker| {
        let backup_env = BackupEnvironment::new(env_type, username.clone(), worker.clone());
        let service = BackupService::new(backup_env, worker.clone());

        let abort_future = worker.abort_future();

        req_body
            .on_upgrade()
            .map_err(Error::from)
            .and_then(move |conn| {
                worker.log("upgrade done");

                let mut http = hyper::server::conn::Http::new();
                http.http2_only(true);

                http.serve_connection(conn, service)
                    .map_err(Error::from)
             })
            .select(abort_future.map_err(|_| {}).then(move |_| { bail!("task aborted"); }))
            .and_then(|(result, _)| Ok(result))
            .map_err(|(err, _)| err)
    })?;

    let response = Response::builder()
        .status(StatusCode::SWITCHING_PROTOCOLS)
        .header(UPGRADE, HeaderValue::from_static(PROXMOX_BACKUP_PROTOCOL_ID))
        .body(Body::empty())?;

    Ok(Box::new(futures::future::ok(response)))
}

fn backup_api() -> Router {

    let test1 = Router::new()
        .get(
            ApiMethod::new(
                test1_get,
                ObjectSchema::new("Test sync callback.")
            )
        );

    let test2 = Router::new()
        .download(
            ApiAsyncMethod::new(
                test2_get,
                ObjectSchema::new("Test async callback.")
            )
        );

    let router = Router::new()
        .subdir("test1", test1)
        .subdir("test2", test2)
        .list_subdirs();

    router
}

fn get_backup_environment(rpcenv: &mut RpcEnvironment) -> &BackupEnvironment  {
    rpcenv.as_any().downcast_ref::<BackupEnvironment>().unwrap()
}

fn test1_get (
    _param: Value,
    _info: &ApiMethod,
    rpcenv: &mut RpcEnvironment,
) -> Result<Value, Error> {

    println!("TYPEID {:?}", (*rpcenv).type_id());

    let env = get_backup_environment(rpcenv);

    env.log("Inside test1_get()");

    Ok(Value::Null)
}

fn test2_get(
    _parts: Parts,
    _req_body: Body,
    _param: Value,
    _info: &ApiAsyncMethod,
    _rpcenv: &mut RpcEnvironment,
) -> Result<BoxFut, Error> {

    let fut = tokio::timer::Interval::new_interval(std::time::Duration::from_millis(300))
        .map_err(|err| http_err!(INTERNAL_SERVER_ERROR, format!("tokio timer interval error: {}", err)))
        .take(50)
        .for_each(|tv| {
            println!("LOOP {:?}", tv);
            Ok(())
        })
        .and_then(|_| {
            println!("TASK DONE");
            Ok(Response::builder()
                .status(StatusCode::OK)
                .body(Body::empty())?)
        });

    Ok(Box::new(fut))
}
