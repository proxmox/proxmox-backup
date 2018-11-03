use failure::*;

use crate::json_schema::*;
use serde_json::{Value};

use std::collections::HashMap;

#[derive(Debug)]
pub struct ApiMethod {
    pub description: &'static str,
    pub parameters: Jss,
    pub returns: Jss,
    pub handler: fn(Value) -> Result<Value, Error>,
}

#[derive(Debug)]
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

    pub fn find_method(&self, components: &[&str]) -> Option<&MethodInfo> {

        if components.len() == 0 { return Some(self); };

        let (dir, rest) = (components[0], &components[1..]);

        if let Some(ref dirmap) = self.subdirs {
            if let Some(ref info) = dirmap.get(dir) {
                return info.find_method(rest);
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
