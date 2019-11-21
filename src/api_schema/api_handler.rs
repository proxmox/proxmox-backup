use failure::Error;
use serde_json::Value;

use hyper::{Body, Response};
use hyper::rt::Future;
use hyper::http::request::Parts;

use super::rpc_environment::RpcEnvironment;
use super::router::ApiMethod;

pub type BoxFut = Box<dyn Future<Output = Result<Response<Body>, failure::Error>> + Send>;

pub type ApiHandlerFn = &'static (
    dyn Fn(Value, &ApiMethod, &mut dyn RpcEnvironment) -> Result<Value, Error>
    + Send + Sync + 'static
);

pub type ApiAsyncHandlerFn = &'static (
    dyn Fn(Parts, Body, Value, &'static ApiMethod, Box<dyn RpcEnvironment>) -> Result<BoxFut, Error>
        + Send + Sync + 'static
);

pub enum ApiHandler {
    Sync(ApiHandlerFn),
    Async(ApiAsyncHandlerFn),
}
