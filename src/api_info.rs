use failure::*;

use crate::json_schema::*;
use serde_json::{Value};

use std::collections::HashMap;

#[derive(Debug)]
pub struct ApiMethod<'a> {
    pub description: &'a str,
    pub parameters: &'a Jss<'a>,
    pub returns: &'a Jss<'a>,
    pub handler: fn(Value) -> Result<Value, Error>,
}

#[derive(Debug)]
pub struct MethodInfo {
    pub get: Option<&'static ApiMethod<'static>>,
    pub put: Option<&'static ApiMethod<'static>>,
    pub post: Option<&'static ApiMethod<'static>>,
    pub delete: Option<&'static ApiMethod<'static>>,
    pub subdirs: Option<HashMap<String, MethodInfo>>,
}

impl MethodInfo {

    pub fn find_method<'a>(&'a self, components: &[&str]) -> Option<&'a MethodInfo> {

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

pub const METHOD_INFO_DEFAULTS: MethodInfo = MethodInfo {
    get: None,
    put: None,
    post: None,
    delete: None,
    subdirs: None,
};

#[macro_export]
macro_rules! methodinfo {
    ($($option:ident => $e:expr),*) => {
        MethodInfo {
            $( $option:  Some($e), )*
            ..METHOD_INFO_DEFAULTS
        };
    }
}
