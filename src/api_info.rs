use std::fmt;
use failure::*;
use futures::future::*;

use crate::json_schema::*;
use serde_json::{Value};
use hyper::{Body, Response, StatusCode};

use std::collections::HashMap;

pub struct ApiMethod {
    pub description: &'static str,
    pub parameters: Jss,
    pub returns: Jss,
    pub handler: fn(Value, &ApiMethod) -> Result<Value, Error>,
    pub async_handler: fn(Value, &ApiMethod) -> Box<Future<Item = Response<Body>, Error = Error> + Send>
}

#[derive(Debug, Fail)]
pub struct ApiError {
    pub code: StatusCode,
    pub message: String,
}

impl ApiError {
    pub fn new(code: StatusCode, message: String) -> Self {
        ApiError { code, message }
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Error {}: {}", self.code, self.message)
    }
}

pub struct MethodInfo {
    pub get: Option<ApiMethod>,
    pub put: Option<ApiMethod>,
    pub post: Option<ApiMethod>,
    pub delete: Option<ApiMethod>,
    pub subdirs: Option<HashMap<String, MethodInfo>>,
}

impl MethodInfo {

    pub fn new() -> Self {
        Self {
            get: None,
            put: None,
            post: None,
            delete: None,
            subdirs: None
        }
    }

    pub fn get(mut self, m: ApiMethod) -> Self {
        self.get = Some(m);
        self
    }

    pub fn find_route(&self, components: &[&str]) -> Option<&MethodInfo> {

        if components.len() == 0 { return Some(self); };

        let (dir, rest) = (components[0], &components[1..]);

        if let Some(ref dirmap) = self.subdirs {
            if let Some(ref info) = dirmap.get(dir) {
                return info.find_route(rest);
            }
        }

        None
    }
}

// fixme: remove - not required?
#[macro_export]
macro_rules! methodinfo {
    ($($option:ident => $e:expr),*) => {{
        let info = MethodInfo::new();

        $(
            info.$option = Some($e);
        )*

        info
    }}
}
