use crate::api::router::*;

use std::collections::HashMap;
use serde_json::Value;

pub struct CliEnvironment {
    result_attributes: HashMap<String, Value>,
}

impl CliEnvironment {
    pub fn new() -> Self {
        Self {  result_attributes: HashMap::new() }
    }
}

impl RpcEnvironment for CliEnvironment {

    fn set_result_attrib(&mut self, name: &str, value: Value) {
        self.result_attributes.insert(name.into(), value);
    }

    fn get_result_attrib(&self, name: &str) -> Option<&Value> {
        self.result_attributes.get(name)
    }

    fn env_type(&self) -> RpcEnvironmentType {
        RpcEnvironmentType::CLI
    }

}
