use failure::*;
use lazy_static::lazy_static;

use std::collections::HashMap;
use std::sync::Arc;

use futures::*;
use hyper::{Body, Request, Response, StatusCode};

use crate::tools;
use crate::api_schema::router::*;
use crate::server::formatter::*;
use crate::server::WorkerTask;

use super::environment::*;

lazy_static!{
    static ref BACKUP_ROUTER: Router = super::backup_api();
}

pub struct BackupService {
    rpcenv: BackupEnvironment,
    worker: Arc<WorkerTask>,
    debug: bool,
}

impl BackupService {

    pub fn new(rpcenv: BackupEnvironment, worker: Arc<WorkerTask>, debug: bool) -> Self {
        Self { rpcenv, worker, debug }
    }

    pub fn debug<S: AsRef<str>>(&self, msg: S) {
        if self.debug { self.worker.log(msg); }
    }

    fn handle_request(&self, req: Request<Body>) -> BoxFut {

        let (parts, body) = req.into_parts();

        let method = parts.method.clone();

        let (path, components) = match tools::normalize_uri_path(parts.uri.path()) {
            Ok((p,c)) => (p, c),
            Err(err) => return Box::new(future::err(http_err!(BAD_REQUEST, err.to_string()))),
        };

        self.debug(format!("{} {}", method, path));

        let mut uri_param = HashMap::new();

        match BACKUP_ROUTER.find_method(&components, method, &mut uri_param) {
            MethodDefinition::None => {
                let err = http_err!(NOT_FOUND, "Path not found.".to_string());
                return Box::new(future::ok((self.rpcenv.formatter.format_error)(err)));
            }
            MethodDefinition::Simple(api_method) => {
                return crate::server::rest::handle_sync_api_request(
                    self.rpcenv.clone(), api_method, self.rpcenv.formatter, parts, body, uri_param);
            }
            MethodDefinition::Async(async_method) => {
                return crate::server::rest::handle_async_api_request(
                    self.rpcenv.clone(), async_method, self.rpcenv.formatter, parts, body, uri_param);
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

impl hyper::service::Service for BackupService {
    type ReqBody = Body;
    type ResBody = Body;
    type Error = Error;
    type Future = Box<Future<Item = Response<Body>, Error = Self::Error> + Send>;

    fn call(&mut self, req: Request<Self::ReqBody>) -> Self::Future {
        let path = req.uri().path().to_owned();
        let method = req.method().clone();
        let worker = self.worker.clone();

        Box::new(self.handle_request(req).then(move |result| {
            match result {
                Ok(res) => {
                    Self::log_response(worker, method, &path, &res);
                    Ok::<_, Error>(res)
                }
                Err(err) => {
                     if let Some(apierr) = err.downcast_ref::<HttpError>() {
                        let mut resp = Response::new(Body::from(apierr.message.clone()));
                        resp.extensions_mut().insert(ErrorMessageExtension(apierr.message.clone()));
                        *resp.status_mut() = apierr.code;
                        Self::log_response(worker, method, &path, &resp);
                        Ok(resp)
                    } else {
                        let mut resp = Response::new(Body::from(err.to_string()));
                        resp.extensions_mut().insert(ErrorMessageExtension(err.to_string()));
                        *resp.status_mut() = StatusCode::BAD_REQUEST;
                        Self::log_response(worker, method, &path, &resp);
                        Ok(resp)
                    }
                }
            }
        }))
    }
}
