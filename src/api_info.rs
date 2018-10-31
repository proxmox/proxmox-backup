use failure::*;

use json_schema::*;
use serde_json::{json, Value};

pub struct ApiMethod {
    pub description: &'static str,
    pub properties: StaticPropertyMap,
    pub returns: Jss,
    pub handler: fn(Value) -> Result<Value, Error>,
}

pub type StaticSubdirMap = phf::Map<&'static str, &'static MethodInfo>;

pub struct MethodInfo {
    pub get: Option<&'static ApiMethod>,
    pub put: Option<&'static ApiMethod>,
    pub post: Option<&'static ApiMethod>,
    pub delete: Option<&'static ApiMethod>,
    pub subdirs: Option<&'static StaticSubdirMap>,
}

pub static METHOD_INFO_DEFAULTS: MethodInfo = MethodInfo {
    get: None,
    put: None,
    post: None,
    delete: None,
    subdirs: None,
};
    
