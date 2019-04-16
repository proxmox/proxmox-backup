use failure::Error;
use serde_json::Value;

use super::router::{ApiMethod, RpcEnvironment};

pub type ApiHandlerFn = Box<
    dyn Fn(Value, &ApiMethod, &mut dyn RpcEnvironment) -> Result<Value, Error>
    + Send + Sync + 'static
>;

pub trait WrapApiHandler<Args, R, MetaArgs> {
    fn wrap(self) -> ApiHandlerFn;
}

// fn()
impl<F, R> WrapApiHandler<(), R, ()> for F
where
    F: Fn() -> Result<R, Error> + Send + Sync + 'static,
    R: serde::Serialize,
{
    fn wrap(self) -> ApiHandlerFn {
        Box::new(move |_value, _method, _rpc_env| {
            Ok(serde_json::to_value((self)()?)?)
        })
    }
}

// fn(Arg)
impl<F, A, R> WrapApiHandler<(A,), R, ()> for F
where
    F: Fn(A) -> Result<R, Error> + Send + Sync + 'static,
    A: serde::de::DeserializeOwned,
    R: serde::Serialize,
{
    fn wrap(self) -> ApiHandlerFn {
        Box::new(move |value, _method, _rpc_env| {
            Ok(serde_json::to_value((self)(serde_json::from_value(value)?)?)?)
        })
    }
}

// fn(&ApiMethod)
impl<F, R> WrapApiHandler<(), R, (ApiMethod,)> for F
where
    F: Fn(&ApiMethod) -> Result<R, Error> + Send + Sync + 'static,
    R: serde::Serialize,
{
    fn wrap(self) -> ApiHandlerFn {
        Box::new(move |_value, method, _rpc_env| {
            Ok(serde_json::to_value((self)(method)?)?)
        })
    }
}

// fn(Arg, &ApiMethod)
impl<F, A, R> WrapApiHandler<(A,), R, (ApiMethod,)> for F
where
    F: Fn(A, &ApiMethod) -> Result<R, Error> + Send + Sync + 'static,
    A: serde::de::DeserializeOwned,
    R: serde::Serialize,
{
    fn wrap(self) -> ApiHandlerFn {
        Box::new(move |value, method, _rpc_env| {
            Ok(serde_json::to_value((self)(
                serde_json::from_value(value)?,
                method,
            )?)?)
        })
    }
}

// RpcEnvironment is a trait, so use a "marker" type for it instead:
pub struct RpcEnvArg();

// fn(&mut dyn RpcEnvironment)
impl<F, R> WrapApiHandler<(), R, (RpcEnvArg,)> for F
where
    F: Fn(&mut dyn RpcEnvironment) -> Result<R, Error> + Send + Sync + 'static,
    R: serde::Serialize,
{
    fn wrap(self) -> ApiHandlerFn {
        Box::new(move |_value, _method, rpc_env| {
            Ok(serde_json::to_value((self)(rpc_env)?)?)
        })
    }
}

// fn(Arg, &mut dyn RpcEnvironment)
impl<F, A, R> WrapApiHandler<(A,), R, (RpcEnvArg,)> for F
where
    F: Fn(A, &mut dyn RpcEnvironment) -> Result<R, Error> + Send + Sync + 'static,
    A: serde::de::DeserializeOwned,
    R: serde::Serialize,
{
    fn wrap(self) -> ApiHandlerFn {
        Box::new(move |value, _method, rpc_env| {
            Ok(serde_json::to_value((self)(
                serde_json::from_value(value)?,
                rpc_env,
            )?)?)
        })
    }
}

// fn(&ApiMethod, &mut dyn RpcEnvironment)
impl<F, R> WrapApiHandler<(), R, (ApiMethod, RpcEnvArg,)> for F
where
    F: Fn(&ApiMethod, &mut dyn RpcEnvironment) -> Result<R, Error> + Send + Sync + 'static,
    R: serde::Serialize,
{
    fn wrap(self) -> ApiHandlerFn {
        Box::new(move |_value, method, rpc_env| {
            Ok(serde_json::to_value((self)(method, rpc_env)?)?)
        })
    }
}

// fn(Arg, &ApiMethod, &mut dyn RpcEnvironment)
impl<F, A, R> WrapApiHandler<(A,), R, (ApiMethod, RpcEnvArg,)> for F
where
    F: Fn(A, &ApiMethod, &mut dyn RpcEnvironment) -> Result<R, Error> + Send + Sync + 'static,
    A: serde::de::DeserializeOwned,
    R: serde::Serialize,
{
    fn wrap(self) -> ApiHandlerFn {
        Box::new(move |value, method, rpc_env| {
            Ok(serde_json::to_value((self)(
                serde_json::from_value(value)?,
                method,
                rpc_env,
            )?)?)
        })
    }
}

