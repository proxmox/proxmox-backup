use serde_json::{json, Value};

use proxmox::api::{RpcEnvironment, RpcEnvironmentType};

/// Encapsulates information about the runtime environment
pub struct RestEnvironment {
    env_type: RpcEnvironmentType,
    result_attributes: Value,
    user: Option<String>,
    client_ip: Option<std::net::SocketAddr>,
}

impl RestEnvironment {
    pub fn new(env_type: RpcEnvironmentType) -> Self {
        Self {
            result_attributes: json!({}),
            user: None,
            client_ip: None,
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

    fn set_client_ip(&mut self, client_ip: Option<std::net::SocketAddr>) {
        self.client_ip = client_ip;
    }

    fn get_client_ip(&self) -> Option<std::net::SocketAddr> {
        self.client_ip.clone()
    }
}
