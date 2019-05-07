use crate::api_schema::router::*;

use std::collections::HashMap;
use serde_json::Value;

/// Encapsulates information about the runtime environment
#[derive(Clone)]
pub struct RestEnvironment {
    env_type: RpcEnvironmentType,
    result_attributes: HashMap<String, Value>,
    user: Option<String>,
}

impl RestEnvironment {
    pub fn new(env_type: RpcEnvironmentType) -> Self {
        Self {
            result_attributes: HashMap::new(),
            user: None,
            env_type,
        }
    }
}

impl RpcEnvironment for RestEnvironment {

    fn set_result_attrib(&mut self, name: &str, value: Value) {
        self.result_attributes.insert(name.into(), value);
    }

    fn get_result_attrib(&self, name: &str) -> Option<&Value> {
        self.result_attributes.get(name)
    }

    fn env_type(&self) -> RpcEnvironmentType {
        self.env_type
    }

    fn set_user(&mut self, user: Option<String>) {
        self.user = user;
    }

    fn get_user(&self) -> Option<String> {
        self.user.clone()
    }
}
