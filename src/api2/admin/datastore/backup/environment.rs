use std::sync::Arc;
use std::collections::HashMap;

use serde_json::Value;

use crate::api_schema::router::{RpcEnvironment, RpcEnvironmentType};
use crate::server::WorkerTask;

/// `RpcEnvironmet` implementation for backup service
#[derive(Clone)]
pub struct BackupEnvironment {
    env_type: RpcEnvironmentType,
    result_attributes: HashMap<String, Value>,
    user: String,
    worker: Arc<WorkerTask>,

}

impl BackupEnvironment {
    pub fn new(env_type: RpcEnvironmentType, user: String, worker: Arc<WorkerTask>) -> Self {
        Self {
            result_attributes: HashMap::new(),
            env_type,
            user,
            worker,
        }
    }

    pub fn log<S: AsRef<str>>(&self, msg: S) {
        self.worker.log(msg);
    }
}

impl RpcEnvironment for BackupEnvironment {

    fn set_result_attrib(&mut self, name: &str, value: Value) {
        self.result_attributes.insert(name.into(), value);
    }

    fn get_result_attrib(&self, name: &str) -> Option<&Value> {
        self.result_attributes.get(name)
    }

    fn env_type(&self) -> RpcEnvironmentType {
        self.env_type
    }

    fn set_user(&mut self, _user: Option<String>) {
        panic!("unable to change user");
    }

    fn get_user(&self) -> Option<String> {
        Some(self.user.clone())
    }
}
