use failure::*;

use json_schema::*;
use serde_json::{Value};

pub struct ApiMethod {
    pub description: &'static str,
    pub properties: StaticPropertyMap,
    pub returns: Jss,
    pub handler: fn(Value) -> Result<Value, Error>,
}

pub type StaticSubdirMap = crate::static_map::StaticMap<'static, &'static str, &'static MethodInfo>;

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

pub fn find_method_info<'a>(root: &'a MethodInfo, components: &[&str]) -> Option<&'a MethodInfo> {

    if components.len() == 0 { return Some(root); };

    let (dir, rest) = (components[0], &components[1..]);

    if let Some(ref dirmap) = root.subdirs {
        if let Some(info) = dirmap.get(&dir) {
            return find_method_info(info, rest);
        }
    }

    None
}
