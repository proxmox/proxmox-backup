use crate::api::router::*;

use std::collections::HashMap;
use serde_json::Value;

pub struct RestEnvironment {
    result_attributes: HashMap<String, Value>,
}

impl RestEnvironment {
    pub fn new() -> Self {
        Self {  result_attributes: HashMap::new() }
    }
}

impl RpcEnvironment for RestEnvironment {

    fn set_result_attrib(&mut self, name: &str, value: Value) {
        self.result_attributes.insert(name.into(), value);
    }

    fn get_result_attrib(&self, name: &str) -> Option<&Value> {
        self.result_attributes.get(name)
    }
}
