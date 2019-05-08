use failure::*;
use lazy_static::lazy_static;

use std::collections::HashMap;
use std::sync::Arc;

use futures::*;
use hyper::header::{HeaderValue, UPGRADE};
use hyper::{Body, Request, Response, StatusCode};
use hyper::http::request::Parts;

use serde_json::Value;

use crate::tools;
use crate::api_schema::router::*;
use crate::api_schema::*;
use crate::server::formatter::*;
use crate::server::WorkerTask;

mod environment;
use environment::*;

pub fn api_method_upgrade_backup() -> ApiAsyncMethod {
    ApiAsyncMethod::new(
        upgrade_h2upload,
        ObjectSchema::new("Upgraded to backup protocol.")
            .required("store", StringSchema::new("Datastore name.")),
    )
}

lazy_static!{
    static ref BACKUP_ROUTER: Router = backup_api();
}


pub struct BackupService {
    rpcenv: BackupEnvironment,
    worker: Arc<WorkerTask>,
}

impl BackupService {

    fn new(rpcenv: BackupEnvironment, worker: Arc<WorkerTask>) -> Self {
        Self { rpcenv, worker }
    }

    fn handle_request(&self, req: Request<Body>) -> BoxFut {

        let (parts, body) = req.into_parts();

        let method = parts.method.clone();

        let (path, components) = match tools::normalize_uri_path(parts.uri.path()) {
            Ok((p,c)) => (p, c),
            Err(err) => return Box::new(future::err(http_err!(BAD_REQUEST, err.to_string()))),
        };

        let formatter = &JSON_FORMATTER;

        self.worker.log(format!("H2 REQUEST {} {}", method, path));
        self.worker.log(format!("H2 COMPO {:?}", components));

        let mut uri_param = HashMap::new();

        match BACKUP_ROUTER.find_method(&components, method, &mut uri_param) {
            MethodDefinition::None => {
                let err = http_err!(NOT_FOUND, "Path not found.".to_string());
                return Box::new(future::ok((formatter.format_error)(err)));
            }
            MethodDefinition::Simple(api_method) => {
                return crate::server::rest::handle_sync_api_request(self.rpcenv.clone(), api_method, formatter, parts, body, uri_param);
            }
            MethodDefinition::Async(async_method) => {
                return crate::server::rest::handle_async_api_request(self.rpcenv.clone(), async_method, formatter, parts, body, uri_param);
            }
        }
    }

    fn log_response(worker: Arc<WorkerTask>, method: hyper::Method, path: &str, resp: &Response<Body>) {

        let status = resp.status();

        if !status.is_success() {
            let reason = status.canonical_reason().unwrap_or("unknown reason");

            let mut message = "request failed";
            if let Some(data) = resp.extensions().get::<ErrorMessageExtension>() {
                message = &data.0;
            }

            worker.log(format!("{} {}: {} {}: {}", method.as_str(), path, status.as_str(), reason, message));
        }
    }
}

impl Drop for  BackupService {
    fn drop(&mut self) {
        println!("SERVER DROP");
    }
}

impl hyper::service::Service for BackupService {
    type ReqBody = Body;
    type ResBody = Body;
    type Error = hyper::Error;
    type Future = Box<Future<Item = Response<Body>, Error = Self::Error> + Send>;

    fn call(&mut self, req: Request<Self::ReqBody>) -> Self::Future {
        let path = req.uri().path().to_owned();
        let method = req.method().clone();
        let worker = self.worker.clone();

        Box::new(self.handle_request(req).then(move |result| {
            match result {
                Ok(res) => {
                    Self::log_response(worker, method, &path, &res);
                    Ok::<_, hyper::Error>(res)
                }
                Err(err) => {
                    if let Some(apierr) = err.downcast_ref::<HttpError>() {
                        let mut resp = Response::new(Body::from(apierr.message.clone()));
                        *resp.status_mut() = apierr.code;
                        Self::log_response(worker, method, &path, &resp);
                        Ok(resp)
                    } else {
                        let mut resp = Response::new(Body::from(err.to_string()));
                        *resp.status_mut() = StatusCode::BAD_REQUEST;
                        Self::log_response(worker, method, &path, &resp);
                        Ok(resp)
                    }
                }
            }
        }))
    }
}

fn upgrade_h2upload(
    parts: Parts,
    req_body: Body,
    _param: Value,
    _info: &ApiAsyncMethod,
    rpcenv: &mut RpcEnvironment,
) -> Result<BoxFut, Error> {
    let expected_protocol: &'static str = "proxmox-backup-protocol-h2";

    let protocols = parts
        .headers
        .get("UPGRADE")
        .ok_or_else(|| format_err!("missing Upgrade header"))?
        .to_str()?;

    if protocols != expected_protocol {
        bail!("invalid protocol name");
    }

    if parts.version >=  http::version::Version::HTTP_2 {
        bail!("unexpected http version '{:?}' (expected version < 2)", parts.version);
    }

    let worker_id = String::from("test2workerid");

    let username = rpcenv.get_user().unwrap();
    let env_type = rpcenv.env_type();

    WorkerTask::spawn("test2_download", Some(worker_id), &username.clone(), true, move |worker| {
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
                    .then(|x| {
                        println!("H2 END");
                        x
                    })
            })
            .select(abort_future.map_err(|_| {}).then(move |_| { bail!("task aborted"); }))
            .and_then(|(result, _)| Ok(result))
            .map_err(|(err, _)| err)
    })?;

    let response = Response::builder()
        .status(StatusCode::SWITCHING_PROTOCOLS)
        .header(UPGRADE, HeaderValue::from_static(expected_protocol))
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
