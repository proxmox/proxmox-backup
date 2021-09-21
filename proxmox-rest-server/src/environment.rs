use std::sync::Arc;
use std::net::SocketAddr;

use serde_json::{json, Value};

use proxmox::api::{RpcEnvironment, RpcEnvironmentType};

use crate::ApiConfig;

/// Encapsulates information about the runtime environment
pub struct RestEnvironment {
    env_type: RpcEnvironmentType,
    result_attributes: Value,
    auth_id: Option<String>,
    client_ip: Option<SocketAddr>,
    api: Arc<ApiConfig>,
}

impl RestEnvironment {
    pub fn new(env_type: RpcEnvironmentType, api: Arc<ApiConfig>) -> Self {
        Self {
            result_attributes: json!({}),
            auth_id: None,
            client_ip: None,
            env_type,
            api,
        }
    }

    pub fn api_config(&self) -> &ApiConfig {
        &self.api
    }

    pub fn log_auth(&self, auth_id: &str) {
        let msg = format!("successful auth for user '{}'", auth_id);
        log::info!("{}", msg);
        if let Some(auth_logger) = self.api.get_auth_log() {
            auth_logger.lock().unwrap().log(&msg);
        }
    }

    pub fn log_failed_auth(&self, failed_auth_id: Option<String>, msg: &str) {
        let msg = match (self.client_ip, failed_auth_id) {
            (Some(peer), Some(user)) => {
                format!("authentication failure; rhost={} user={} msg={}", peer, user, msg)
            }
            (Some(peer), None) => {
                format!("authentication failure; rhost={} msg={}", peer, msg)
            }
            (None, Some(user)) => {
                format!("authentication failure; rhost=unknown user={} msg={}", user, msg)
            }
            (None, None) => {
                format!("authentication failure; rhost=unknown msg={}", msg)
            }
        };
        log::error!("{}", msg);
        if let Some(auth_logger) = self.api.get_auth_log() {
            auth_logger.lock().unwrap().log(&msg);
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

    fn set_auth_id(&mut self, auth_id: Option<String>) {
        self.auth_id = auth_id;
    }

    fn get_auth_id(&self) -> Option<String> {
        self.auth_id.clone()
    }

    fn set_client_ip(&mut self, client_ip: Option<SocketAddr>) {
        self.client_ip = client_ip;
    }

    fn get_client_ip(&self) -> Option<SocketAddr> {
        self.client_ip
    }
}
