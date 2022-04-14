use std::collections::HashSet;
use std::sync::{Arc, RwLock};

use serde_json::{json, Value};

use proxmox_router::{RpcEnvironment, RpcEnvironmentType};

use pbs_api_types::Authid;
use pbs_datastore::backup_info::BackupDir;
use pbs_datastore::DataStore;
use proxmox_rest_server::formatter::*;
use proxmox_rest_server::WorkerTask;

/// `RpcEnvironmet` implementation for backup reader service
#[derive(Clone)]
pub struct ReaderEnvironment {
    env_type: RpcEnvironmentType,
    result_attributes: Value,
    auth_id: Authid,
    pub debug: bool,
    pub formatter: &'static dyn OutputFormatter,
    pub worker: Arc<WorkerTask>,
    pub datastore: Arc<DataStore>,
    pub backup_dir: BackupDir,
    allowed_chunks: Arc<RwLock<HashSet<[u8; 32]>>>,
}

impl ReaderEnvironment {
    pub fn new(
        env_type: RpcEnvironmentType,
        auth_id: Authid,
        worker: Arc<WorkerTask>,
        datastore: Arc<DataStore>,
        backup_dir: BackupDir,
    ) -> Self {
        Self {
            result_attributes: json!({}),
            env_type,
            auth_id,
            worker,
            datastore,
            debug: false,
            formatter: JSON_FORMATTER,
            backup_dir,
            allowed_chunks: Arc::new(RwLock::new(HashSet::new())),
        }
    }

    pub fn log<S: AsRef<str>>(&self, msg: S) {
        self.worker.log_message(msg);
    }

    pub fn debug<S: AsRef<str>>(&self, msg: S) {
        if self.debug {
            self.worker.log_message(msg);
        }
    }

    pub fn register_chunk(&self, digest: [u8; 32]) {
        let mut allowed_chunks = self.allowed_chunks.write().unwrap();
        allowed_chunks.insert(digest);
    }

    pub fn check_chunk_access(&self, digest: [u8; 32]) -> bool {
        self.allowed_chunks.read().unwrap().contains(&digest)
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

    fn set_auth_id(&mut self, _auth_id: Option<String>) {
        panic!("unable to change auth_id");
    }

    fn get_auth_id(&self) -> Option<String> {
        Some(self.auth_id.to_string())
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
