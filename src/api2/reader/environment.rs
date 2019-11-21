//use failure::*;
use std::sync::Arc;
use std::collections::HashMap;

use serde_json::Value;

use proxmox::api::{RpcEnvironment, RpcEnvironmentType};

use crate::server::WorkerTask;
use crate::backup::*;
use crate::server::formatter::*;

//use proxmox::tools;

/// `RpcEnvironmet` implementation for backup reader service
#[derive(Clone)]
pub struct ReaderEnvironment {
    env_type: RpcEnvironmentType,
    result_attributes: HashMap<String, Value>,
    user: String,
    pub debug: bool,
    pub formatter: &'static OutputFormatter,
    pub worker: Arc<WorkerTask>,
    pub datastore: Arc<DataStore>,
    pub backup_dir: BackupDir,
    // state: Arc<Mutex<SharedBackupState>>
}

impl ReaderEnvironment {
    pub fn new(
        env_type: RpcEnvironmentType,
        user: String,
        worker: Arc<WorkerTask>,
        datastore: Arc<DataStore>,
        backup_dir: BackupDir,
    ) -> Self {


        Self {
            result_attributes: HashMap::new(),
            env_type,
            user,
            worker,
            datastore,
            debug: false,
            formatter: &JSON_FORMATTER,
            backup_dir,
            //state: Arc::new(Mutex::new(state)),
        }
    }

    pub fn log<S: AsRef<str>>(&self, msg: S) {
        self.worker.log(msg);
    }

    pub fn debug<S: AsRef<str>>(&self, msg: S) {
        if self.debug { self.worker.log(msg); }
    }

}

impl RpcEnvironment for ReaderEnvironment {

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

impl AsRef<ReaderEnvironment> for dyn RpcEnvironment {
    fn as_ref(&self) -> &ReaderEnvironment {
        self.as_any().downcast_ref::<ReaderEnvironment>().unwrap()
    }
}

impl AsRef<ReaderEnvironment> for Box<dyn RpcEnvironment> {
    fn as_ref(&self) -> &ReaderEnvironment {
        self.as_any().downcast_ref::<ReaderEnvironment>().unwrap()
    }
}
