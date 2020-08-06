//use anyhow::{bail, format_err, Error};
use std::sync::Arc;

use serde_json::{json, Value};

use proxmox::api::{RpcEnvironment, RpcEnvironmentType};

use crate::api2::types::Userid;
use crate::backup::*;
use crate::server::formatter::*;
use crate::server::WorkerTask;

//use proxmox::tools;

/// `RpcEnvironmet` implementation for backup reader service
#[derive(Clone)]
pub struct ReaderEnvironment {
    env_type: RpcEnvironmentType,
    result_attributes: Value,
    user: Userid,
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
        user: Userid,
        worker: Arc<WorkerTask>,
        datastore: Arc<DataStore>,
        backup_dir: BackupDir,
    ) -> Self {


        Self {
            result_attributes: json!({}),
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

    fn result_attrib_mut(&mut self) -> &mut Value {
        &mut self.result_attributes
    }

    fn result_attrib(&self) -> &Value {
        &self.result_attributes
    }

    fn env_type(&self) -> RpcEnvironmentType {
        self.env_type
    }

    fn set_user(&mut self, _user: Option<String>) {
        panic!("unable to change user");
    }

    fn get_user(&self) -> Option<String> {
        Some(self.user.to_string())
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
