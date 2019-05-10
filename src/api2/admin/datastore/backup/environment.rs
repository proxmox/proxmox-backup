use failure::*;
use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use std::path::PathBuf;

use serde_json::Value;

use crate::api_schema::router::{RpcEnvironment, RpcEnvironmentType};
use crate::server::WorkerTask;
use crate::backup::*;
use crate::server::formatter::*;
use hyper::{Body, Response};

struct SharedBackupState {
    uid_counter: usize,
    dynamic_writers: HashMap<usize, (u64 /* offset */, DynamicIndexWriter)>,
}

/// `RpcEnvironmet` implementation for backup service
#[derive(Clone)]
pub struct BackupEnvironment {
    env_type: RpcEnvironmentType,
    result_attributes: HashMap<String, Value>,
    user: String,
    pub formatter: &'static OutputFormatter,
    pub worker: Arc<WorkerTask>,
    pub datastore: Arc<DataStore>,
    pub backup_dir: BackupDir,
    pub path: PathBuf,
    state: Arc<Mutex<SharedBackupState>>
}

impl BackupEnvironment {
    pub fn new(
        env_type: RpcEnvironmentType,
        user: String,
        worker: Arc<WorkerTask>,
        datastore: Arc<DataStore>,
        backup_dir: BackupDir,
        path: PathBuf,
    ) -> Self {

        let state = SharedBackupState {
            uid_counter: 0,
            dynamic_writers: HashMap::new(),
        };

        Self {
            result_attributes: HashMap::new(),
            env_type,
            user,
            worker,
            datastore,
            formatter: &JSON_FORMATTER,
            backup_dir,
            path,
            state: Arc::new(Mutex::new(state)),
        }
    }

    /// Get an unique integer ID
    pub fn next_uid(&self) -> usize {
        let mut state = self.state.lock().unwrap();
        state.uid_counter += 1;
        state.uid_counter
    }

    /// Store the writer with an unique ID
    pub fn register_dynamic_writer(&self, writer: DynamicIndexWriter) -> usize {
       let mut state = self.state.lock().unwrap();
        state.uid_counter += 1;
        let uid = state.uid_counter;

        state.dynamic_writers.insert(uid, (0, writer));
        uid
    }

    /// Append chunk to dynamic writer
    pub fn dynamic_writer_append_chunk(&self, wid: usize, size: u64, digest: &[u8; 32]) -> Result<(), Error> {
        let mut state = self.state.lock().unwrap();

        let mut data = match state.dynamic_writers.get_mut(&wid) {
            Some(data) => data,
            None => bail!("dynamic writer '{}' not registered", wid),
        };

        data.0 += size;

        data.1.add_chunk(data.0, digest)?;

        Ok(())
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
