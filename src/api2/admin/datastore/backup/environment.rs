use failure::*;
use std::sync::{Arc, Mutex};
use std::collections::HashMap;

use serde_json::Value;

use crate::api_schema::router::{RpcEnvironment, RpcEnvironmentType};
use crate::server::WorkerTask;
use crate::backup::*;
use crate::server::formatter::*;
use hyper::{Body, Response};

struct SharedBackupState {
    finished: bool,
    uid_counter: usize,
    dynamic_writers: HashMap<usize, (u64 /* offset */, DynamicIndexWriter)>,
}

impl SharedBackupState {

    // Raise error if finished flag is set
    fn ensure_unfinished(&self) -> Result<(), Error> {
        if self.finished {
            bail!("backup already marked as finished.");
        }
        Ok(())
    }

    // Get an unique integer ID
    pub fn next_uid(&mut self) -> usize {
        self.uid_counter += 1;
        self.uid_counter
    }
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
    pub last_backup: Option<BackupInfo>,
    state: Arc<Mutex<SharedBackupState>>
}

impl BackupEnvironment {
    pub fn new(
        env_type: RpcEnvironmentType,
        user: String,
        worker: Arc<WorkerTask>,
        datastore: Arc<DataStore>,
        backup_dir: BackupDir,
    ) -> Self {

        let state = SharedBackupState {
            finished: false,
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
            last_backup: None,
            state: Arc::new(Mutex::new(state)),
        }
    }

    /// Store the writer with an unique ID
    pub fn register_dynamic_writer(&self, writer: DynamicIndexWriter) -> Result<usize, Error> {
        let mut state = self.state.lock().unwrap();

        state.ensure_unfinished()?;

        let uid = state.next_uid();

        state.dynamic_writers.insert(uid, (0, writer));

        Ok(uid)
    }

    /// Append chunk to dynamic writer
    pub fn dynamic_writer_append_chunk(&self, wid: usize, size: u64, digest: &[u8; 32]) -> Result<(), Error> {
        let mut state = self.state.lock().unwrap();

        state.ensure_unfinished()?;

        let mut data = match state.dynamic_writers.get_mut(&wid) {
            Some(data) => data,
            None => bail!("dynamic writer '{}' not registered", wid),
        };

        data.0 += size;

        data.1.add_chunk(data.0, digest)?;

        Ok(())
    }

    /// Close dynamic writer
    pub fn dynamic_writer_close(&self, wid: usize) -> Result<(), Error> {
        let mut state = self.state.lock().unwrap();

        state.ensure_unfinished()?;

        let mut data = match state.dynamic_writers.remove(&wid) {
            Some(data) => data,
            None => bail!("dynamic writer '{}' not registered", wid),
        };

        data.1.close()?;

        Ok(())
    }

    /// Mark backup as finished
    pub fn finish_backup(&self) -> Result<(), Error> {
        let mut state = self.state.lock().unwrap();
        // test if all writer are correctly closed

        state.ensure_unfinished()?;

        state.finished = true;

        if state.dynamic_writers.len() != 0 {
            bail!("found open index writer - unable to finish backup");
        }

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

    /// Raise error if finished flag is not set
    pub fn ensure_finished(&self) -> Result<(), Error> {
        let state = self.state.lock().unwrap();
        if !state.finished {
            bail!("backup ended but finished flag is not set.");
        }
        Ok(())
    }

    /// Remove complete backup
    pub fn remove_backup(&self) -> Result<(), Error> {
        let mut state = self.state.lock().unwrap();
        state.finished = true;

        self.datastore.remove_backup_dir(&self.backup_dir)?;

        Ok(())
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
