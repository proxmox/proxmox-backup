use failure::*;
use std::sync::Arc;
use std::collections::HashMap;

use serde_json::Value;

use crate::api_schema::router::{RpcEnvironment, RpcEnvironmentType};
use crate::server::WorkerTask;
use crate::backup::*;
use crate::server::formatter::*;
use hyper::{Body, Response};

/// `RpcEnvironmet` implementation for backup service
#[derive(Clone)]
pub struct BackupEnvironment {
    env_type: RpcEnvironmentType,
    result_attributes: HashMap<String, Value>,
    user: String,
    pub formatter: &'static OutputFormatter,
    pub worker: Arc<WorkerTask>,
    pub datastore: Arc<DataStore>,
}

impl BackupEnvironment {
    pub fn new(env_type: RpcEnvironmentType, user: String, worker: Arc<WorkerTask>, datastore: Arc<DataStore>) -> Self {
        Self {
            result_attributes: HashMap::new(),
            env_type,
            user,
            worker,
            datastore,
            formatter: &JSON_FORMATTER,
        }
    }

    pub fn log<S: AsRef<str>>(&self, msg: S) {
        self.worker.log(msg);
    }

    pub fn format_response(&self, result: Result<Value, Error>) -> Response<Body> {
        match result {
            Ok(data) => (self.formatter.format_data)(data, self),
            Err(err) => (self.formatter.format_error)(err),
        }
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

impl AsRef<BackupEnvironment> for RpcEnvironment {
    fn as_ref(&self) -> &BackupEnvironment {
        self.as_any().downcast_ref::<BackupEnvironment>().unwrap()
    }
}
impl AsRef<BackupEnvironment> for Box<RpcEnvironment> {
    fn as_ref(&self) -> &BackupEnvironment {
        self.as_any().downcast_ref::<BackupEnvironment>().unwrap()
    }
}
