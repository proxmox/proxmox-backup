use anyhow::Error;

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use futures::*;
use hyper::{Body, Request, Response, StatusCode};

use proxmox_router::http_err;
use proxmox_router::{ApiResponseFuture, HttpError, Router, RpcEnvironment};

use crate::formatter::*;
use crate::{normalize_uri_path, WorkerTask};

/// Hyper Service implementation to handle stateful H2 connections.
///
/// We use this kind of service to handle backup protocol
/// connections. State is stored inside the generic ``rpcenv``. Logs
/// goes into the ``WorkerTask`` log.
pub struct H2Service<E> {
    router: &'static Router,
    rpcenv: E,
    worker: Arc<WorkerTask>,
    debug: bool,
}

impl<E: RpcEnvironment + Clone> H2Service<E> {
    pub fn new(rpcenv: E, worker: Arc<WorkerTask>, router: &'static Router, debug: bool) -> Self {
        Self {
            rpcenv,
            worker,
            router,
            debug,
        }
    }

    pub fn debug<S: AsRef<str>>(&self, msg: S) {
        if self.debug {
            self.worker.log_message(msg);
        }
    }

    fn handle_request(&self, req: Request<Body>) -> ApiResponseFuture {
        let (parts, body) = req.into_parts();

        let method = parts.method.clone();

        let (path, components) = match normalize_uri_path(parts.uri.path()) {
            Ok((p, c)) => (p, c),
            Err(err) => return future::err(http_err!(BAD_REQUEST, "{}", err)).boxed(),
        };

        self.debug(format!("{} {}", method, path));

        let mut uri_param = HashMap::new();

        let formatter = JSON_FORMATTER;

        match self.router.find_method(&components, method, &mut uri_param) {
            None => {
                let err = http_err!(NOT_FOUND, "Path '{}' not found.", path);
                future::ok(formatter.format_error(err)).boxed()
            }
            Some(api_method) => crate::rest::handle_api_request(
                self.rpcenv.clone(),
                api_method,
                formatter,
                parts,
                body,
                uri_param,
            )
            .boxed(),
        }
    }

    fn log_response(
        worker: Arc<WorkerTask>,
        method: hyper::Method,
        path: &str,
        resp: &Response<Body>,
    ) {
        let status = resp.status();

        if !status.is_success() {
            let reason = status.canonical_reason().unwrap_or("unknown reason");

            let mut message = "request failed";
            if let Some(data) = resp.extensions().get::<ErrorMessageExtension>() {
                message = &data.0;
            }

            worker.log_message(format!(
                "{} {}: {} {}: {}",
                method.as_str(),
                path,
                status.as_str(),
                reason,
                message
            ));
        }
    }
}

impl<E: RpcEnvironment + Clone> tower_service::Service<Request<Body>> for H2Service<E> {
    type Response = Response<Body>;
    type Error = Error;
    #[allow(clippy::type_complexity)]
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        let path = req.uri().path().to_owned();
        let method = req.method().clone();
        let worker = self.worker.clone();

        self.handle_request(req)
            .map(move |result| match result {
                Ok(res) => {
                    Self::log_response(worker, method, &path, &res);
                    Ok::<_, Error>(res)
                }
                Err(err) => {
                    if let Some(apierr) = err.downcast_ref::<HttpError>() {
                        let mut resp = Response::new(Body::from(apierr.message.clone()));
                        resp.extensions_mut()
                            .insert(ErrorMessageExtension(apierr.message.clone()));
                        *resp.status_mut() = apierr.code;
                        Self::log_response(worker, method, &path, &resp);
                        Ok(resp)
                    } else {
                        let mut resp = Response::new(Body::from(err.to_string()));
                        resp.extensions_mut()
                            .insert(ErrorMessageExtension(err.to_string()));
                        *resp.status_mut() = StatusCode::BAD_REQUEST;
                        Self::log_response(worker, method, &path, &resp);
                        Ok(resp)
                    }
                }
            })
            .boxed()
    }
}
