use failure::*;

use serde_json::Value;
use std::collections::HashMap;
use std::fmt;

use hyper::{Method, StatusCode};
//use hyper::http::request::Parts;

use super::schema::*;
pub use super::rpc_environment::*;
pub use super::api_handler::*;


#[derive(Debug, Fail)]
pub struct HttpError {
    pub code: StatusCode,
    pub message: String,
}

impl HttpError {
    pub fn new(code: StatusCode, message: String) -> Self {
        HttpError { code, message }
    }
}

impl fmt::Display for HttpError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

#[macro_export]
macro_rules! http_err {
    ($status:ident, $msg:expr) => {{
        Error::from(HttpError::new(StatusCode::$status, $msg))
    }}
}

/// This struct defines synchronous API call which returns the restulkt as json `Value`
pub struct ApiMethod {
    /// The protected flag indicates that the provides function should be forwarded
    /// to the deaemon running in priviledged mode.
    pub protected: bool,
    /// This flag indicates that the provided method may change the local timezone, so the server
    /// should do a tzset afterwards
    pub reload_timezone: bool,
    /// Parameter type Schema
    pub parameters: &'static ObjectSchema,
    /// Return type Schema
    pub returns: &'static Schema,
    /// Handler function
    pub handler: &'static ApiHandler,
}

impl std::fmt::Debug for ApiMethod {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "ApiMethod {{ ")?;
        write!(f, "  parameters: {:?}", self.parameters)?;
        write!(f, "  returns: {:?}", self.returns)?;
        write!(f, "  handler: {:p}", &self.handler)?;
        write!(f, "}}")
    }
}

const NULL_SCHEMA: Schema = Schema::Null;

fn dummy_handler_fn(_arg: Value, _method: &ApiMethod, _env: &mut dyn RpcEnvironment) -> Result<Value, Error> {
    // do nothing
    Ok(Value::Null)
}

const DUMMY_HANDLER: ApiHandler = ApiHandler::Sync(&dummy_handler_fn);

impl ApiMethod {

    pub const fn new(handler: &'static ApiHandler, parameters: &'static ObjectSchema) -> Self {
        Self {
            parameters,
            handler,
            returns: &NULL_SCHEMA,
            protected: false,
            reload_timezone: false,
        }
    }

    pub const fn new_dummy(parameters: &'static ObjectSchema) -> Self {
        Self {
            parameters,
            handler: &DUMMY_HANDLER,
            returns: &NULL_SCHEMA,
            protected: false,
            reload_timezone: false,
        }
    }

    pub const fn returns(mut self, schema: &'static Schema) -> Self {

        self.returns = schema;

        self
    }

    pub const fn protected(mut self, protected: bool) -> Self {

        self.protected = protected;

        self
    }

    pub const fn reload_timezone(mut self, reload_timezone: bool) -> Self {

        self.reload_timezone = reload_timezone;

        self
    }
}

pub type SubdirMap = &'static [(&'static str, &'static Router)];

pub enum SubRoute {
    //Hash(HashMap<String, Router>),
    Map(SubdirMap),
    MatchAll { router: &'static Router, param_name: &'static str },
}

/// Macro to create an ApiMethod to list entries from SubdirMap
#[macro_export]
macro_rules! list_subdirs_api_method {
    ($map:expr) => {
        ApiMethod::new(
            &ApiHandler::Sync( & |_, _, _| {
                let index = serde_json::json!(
                    $map.iter().map(|s| serde_json::json!({ "subdir": s.0}))
                        .collect::<Vec<serde_json::Value>>()
                );
                Ok(index)
            }),
            &crate::api_schema::ObjectSchema::new("Directory index.", &[]).additional_properties(true)
        )
    }
}

pub struct Router {
    pub get: Option<&'static ApiMethod>,
    pub put: Option<&'static ApiMethod>,
    pub post: Option<&'static ApiMethod>,
    pub delete: Option<&'static ApiMethod>,
    pub subroute: Option<SubRoute>,
}

impl Router {

    pub const fn new() -> Self {
        Self {
            get: None,
            put: None,
            post: None,
            delete: None,
            subroute: None,
        }
    }

    pub const fn subdirs(mut self, map: SubdirMap) -> Self {
        self.subroute = Some(SubRoute::Map(map));
        self
    }

    pub const fn match_all(mut self, param_name: &'static str, router: &'static Router) -> Self {
        self.subroute = Some(SubRoute::MatchAll { router, param_name });
        self
    }
   
    pub const fn get(mut self, m: &'static ApiMethod) -> Self {
        self.get = Some(m);
        self
    }

    pub const fn put(mut self, m: &'static ApiMethod) -> Self {
        self.put = Some(m);
        self
    }

    pub const fn post(mut self, m: &'static ApiMethod) -> Self {
        self.post = Some(m);
        self
    }

    /// Same as post, buth async (fixme: expect Async)
    pub const fn upload(mut self, m: &'static ApiMethod) -> Self {
        self.post = Some(m);
        self
    }

    /// Same as get, but async (fixme: expect Async)
    pub const fn download(mut self, m: &'static ApiMethod) -> Self {
        self.get = Some(m);
        self
    }

    /// Same as get, but async (fixme: expect Async)
    pub const fn upgrade(mut self, m: &'static ApiMethod) -> Self {
        self.get = Some(m);
        self
    }

    pub const fn delete(mut self, m: &'static ApiMethod) -> Self {
        self.delete = Some(m);
        self
    }

    pub fn find_route(&self, components: &[&str], uri_param: &mut HashMap<String, String>) -> Option<&Router> {

        if components.is_empty() { return Some(self); };

        let (dir, rest) = (components[0], &components[1..]);

        match self.subroute {
            None => {},
            Some(SubRoute::Map(dirmap)) => {
                if let Ok(ind) = dirmap.binary_search_by_key(&dir, |(name, _)| name) {
                    let (_name, router) = dirmap[ind];
                    //println!("FOUND SUBDIR {}", dir);
                    return router.find_route(rest, uri_param);
                }
            }
            Some(SubRoute::MatchAll { router, param_name }) => {
                //println!("URI PARAM {} = {}", param_name, dir); // fixme: store somewhere
                uri_param.insert(param_name.to_owned(), dir.into());
                return router.find_route(rest, uri_param);
            },
        }

        None
    }

    pub fn find_method(
        &self,
        components: &[&str],
        method: Method,
        uri_param: &mut HashMap<String, String>
    ) -> Option<&ApiMethod> {

        if let Some(info) = self.find_route(components, uri_param) {
            return match method {
                Method::GET => info.get,
                Method::PUT => info.put,
                Method::POST => info.post,
                Method::DELETE => info.delete,
                _ => None,
            };
        }
        None
    }
}

impl Default for Router {
    fn default() -> Self {
        Self::new()
    }
}
