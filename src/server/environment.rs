use serde_json::{json, Value};

use proxmox::api::{RpcEnvironment, RpcEnvironmentType};

/// Encapsulates information about the runtime environment
pub struct RestEnvironment {
    env_type: RpcEnvironmentType,
    result_attributes: Value,
    user: Option<String>,
}

impl RestEnvironment {
    pub fn new(env_type: RpcEnvironmentType) -> Self {
        Self {
            result_attributes: json!({}),
            user: None,
            env_type,
        }
    }
}

impl RpcEnvironment for RestEnvironment {

    fn result_attrib_mut (&mut self) -> &mut Value {
        &mut self.result_attributes
    }

    fn result_attrib(&self) -> &Value {
        &self.result_attributes
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
