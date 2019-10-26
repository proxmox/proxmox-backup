use failure::*;

use crate::api_schema::*;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use std::fmt;

use hyper::{Body, Method, Response, StatusCode};
use hyper::rt::Future;
use hyper::http::request::Parts;

use super::api_handler::*;

pub type BoxFut = Box<dyn Future<Output = Result<Response<Body>, failure::Error>> + Send>;

/// Abstract Interface for API methods to interact with the environment
pub trait RpcEnvironment: std::any::Any + crate::tools::AsAny + Send {

    /// Use this to pass additional result data. It is up to the environment
    /// how the data is used.
    fn set_result_attrib(&mut self, name: &str, value: Value);

    /// Query additional result data.
    fn get_result_attrib(&self, name: &str) -> Option<&Value>;

    /// The environment type
    fn env_type(&self) -> RpcEnvironmentType;

    /// Set user name
    fn set_user(&mut self, user: Option<String>);

    /// Get user name
    fn get_user(&self) -> Option<String>;
}


/// Environment Type
///
/// We use this to enumerate the different environment types. Some methods
/// needs to do different things when started from the command line interface,
/// or when executed from a privileged server running as root.
#[derive(PartialEq, Copy, Clone)]
pub enum RpcEnvironmentType {
    /// Command started from command line
    CLI,
    /// Access from public accessible server
    PUBLIC,
    /// Access from privileged server (run as root)
    PRIVILEGED,
}

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

macro_rules! http_err {
    ($status:ident, $msg:expr) => {{
        Error::from(HttpError::new(StatusCode::$status, $msg))
    }}
}

type ApiAsyncHandlerFn = Box<
    dyn Fn(Parts, Body, Value, &ApiAsyncMethod, Box<dyn RpcEnvironment>) -> Result<BoxFut, Error>
    + Send + Sync + 'static
>;

/// This struct defines synchronous API call which returns the restulkt as json `Value`
pub struct ApiMethod {
    /// The protected flag indicates that the provides function should be forwarded
    /// to the deaemon running in priviledged mode.
    pub protected: bool,
    /// This flag indicates that the provided method may change the local timezone, so the server
    /// should do a tzset afterwards
    pub reload_timezone: bool,
    /// Parameter type Schema
    pub parameters: ObjectSchema,
    /// Return type Schema
    pub returns: Arc<Schema>,
    /// Handler function
    pub handler: Option<ApiHandlerFn>,
}

impl ApiMethod {

    pub fn new<F, Args, R, MetaArgs>(func: F, parameters: ObjectSchema) -> Self
    where
        F: WrapApiHandler<Args, R, MetaArgs>,
    {
        Self {
            parameters,
            handler: Some(func.wrap()),
            returns: Arc::new(Schema::Null),
            protected: false,
            reload_timezone: false,
        }
    }

    pub fn new_dummy(parameters: ObjectSchema) -> Self {
        Self {
            parameters,
            handler: None,
            returns: Arc::new(Schema::Null),
            protected: false,
            reload_timezone: false,
        }
    }

    pub fn returns<S: Into<Arc<Schema>>>(mut self, schema: S) -> Self {

        self.returns = schema.into();

        self
    }

    pub fn protected(mut self, protected: bool) -> Self {

        self.protected = protected;

        self
    }

    pub fn reload_timezone(mut self, reload_timezone: bool) -> Self {

        self.reload_timezone = reload_timezone;

        self
    }
}

pub struct ApiAsyncMethod {
    pub parameters: ObjectSchema,
    pub returns: Arc<Schema>,
    pub handler: ApiAsyncHandlerFn,
}

impl ApiAsyncMethod {

    pub fn new<F>(handler: F, parameters: ObjectSchema) -> Self
    where
        F: Fn(Parts, Body, Value, &ApiAsyncMethod, Box<dyn RpcEnvironment>) -> Result<BoxFut, Error>
            + Send + Sync + 'static,
    {
        Self {
            parameters,
            handler: Box::new(handler),
            returns: Arc::new(Schema::Null),
        }
    }

    pub fn returns<S: Into<Arc<Schema>>>(mut self, schema: S) -> Self {

        self.returns = schema.into();

        self
    }
}

pub enum SubRoute {
    None,
    Hash(HashMap<String, Router>),
    MatchAll { router: Box<Router>, param_name: String },
}

pub enum MethodDefinition {
    None,
    Simple(ApiMethod),
    Async(ApiAsyncMethod),
}

pub struct Router {
    pub get: MethodDefinition,
    pub put: MethodDefinition,
    pub post: MethodDefinition,
    pub delete: MethodDefinition,
    pub subroute: SubRoute,
}

impl Router {

    pub fn new() -> Self {
        Self {
            get: MethodDefinition::None,
            put: MethodDefinition::None,
            post: MethodDefinition::None,
            delete: MethodDefinition::None,
            subroute: SubRoute::None
        }
    }

    pub fn subdir<S: Into<String>>(mut self, subdir: S, router: Router) -> Self {
        if let SubRoute::None = self.subroute {
            self.subroute = SubRoute::Hash(HashMap::new());
        }
        match self.subroute {
            SubRoute::Hash(ref mut map) => {
                map.insert(subdir.into(), router);
            }
            _ => panic!("unexpected subroute type"),
        }
        self
    }

    pub fn subdirs(mut self, map: HashMap<String, Router>) -> Self {
        self.subroute = SubRoute::Hash(map);
        self
    }

    pub fn match_all<S: Into<String>>(mut self, param_name: S, router: Router) -> Self {
        if let SubRoute::None = self.subroute {
            self.subroute = SubRoute::MatchAll { router: Box::new(router), param_name: param_name.into() };
        } else {
            panic!("unexpected subroute type");
        }
        self
    }

    pub fn list_subdirs(self) -> Self {
        match self.get {
            MethodDefinition::None => {},
            _ => panic!("cannot create directory index - method get already in use"),
        }
        match self.subroute {
            SubRoute::Hash(ref map) => {
                let index = json!(map.keys().map(|s| json!({ "subdir": s}))
                    .collect::<Vec<Value>>());
                self.get(ApiMethod::new(
                    move || { Ok(index.clone()) },
                    ObjectSchema::new("Directory index.").additional_properties(true))
                )
            }
            _ => panic!("cannot create directory index (no SubRoute::Hash)"),
        }
    }

    pub fn get(mut self, m: ApiMethod) -> Self {
        self.get = MethodDefinition::Simple(m);
        self
    }

    pub fn put(mut self, m: ApiMethod) -> Self {
        self.put = MethodDefinition::Simple(m);
        self
    }

    pub fn post(mut self, m: ApiMethod) -> Self {
        self.post = MethodDefinition::Simple(m);
        self
    }

    pub fn upload(mut self, m: ApiAsyncMethod) -> Self {
        self.post = MethodDefinition::Async(m);
        self
    }

    pub fn download(mut self, m: ApiAsyncMethod) -> Self {
        self.get = MethodDefinition::Async(m);
        self
    }

    pub fn upgrade(mut self, m: ApiAsyncMethod) -> Self {
        self.get = MethodDefinition::Async(m);
        self
    }

    pub fn delete(mut self, m: ApiMethod) -> Self {
        self.delete = MethodDefinition::Simple(m);
        self
    }

    pub fn find_route(&self, components: &[&str], uri_param: &mut HashMap<String, String>) -> Option<&Router> {

        if components.is_empty() { return Some(self); };

        let (dir, rest) = (components[0], &components[1..]);

        match self.subroute {
            SubRoute::None => {},
            SubRoute::Hash(ref dirmap) => {
                if let Some(ref router) = dirmap.get(dir) {
                    //println!("FOUND SUBDIR {}", dir);
                    return router.find_route(rest, uri_param);
                }
            }
            SubRoute::MatchAll { ref router, ref param_name } => {
                //println!("URI PARAM {} = {}", param_name, dir); // fixme: store somewhere
                uri_param.insert(param_name.clone(), dir.into());
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
    ) -> &MethodDefinition {

        if let Some(info) = self.find_route(components, uri_param) {
            return match method {
                Method::GET => &info.get,
                Method::PUT => &info.put,
                Method::POST => &info.post,
                Method::DELETE => &info.delete,
                _ => &MethodDefinition::None,
            };
        }
        &MethodDefinition::None
    }
}

impl Default for Router {
    fn default() -> Self {
        Self::new()
    }
}
