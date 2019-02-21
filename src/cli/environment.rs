use crate::api_schema::router::*;

use std::collections::HashMap;
use serde_json::Value;

/// `RpcEnvironmet` implementation for command line tools
pub struct CliEnvironment {
    result_attributes: HashMap<String, Value>,
    user: Option<String>,
}

impl CliEnvironment {
    pub fn new() -> Self {
        Self {
            result_attributes: HashMap::new(),
            user: None,
        }
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

    fn set_user(&mut self, user: Option<String>) {
        self.user = user;
    }

    fn get_user(&self) -> Option<String> {
        self.user.clone()
    }
}
