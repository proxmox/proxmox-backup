use anyhow::{bail, Error};
use serde_json::Value;
use ::serde::{Deserialize, Serialize};

use proxmox::api::{api, Router, RpcEnvironment};
use proxmox::tools::fs::open_file_locked;

use crate::api2::types::*;
use crate::config::sync::{self, SyncJobConfig};

// fixme: add access permissions

#[api(
    input: {
        properties: {},
    },
    returns: {
        description: "List configured jobs.",
        type: Array,
        items: { type: sync::SyncJobConfig },
    },
)]
/// List all sync jobs
pub fn list_sync_jobs(
    _param: Value,
    mut rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<SyncJobConfig>, Error> {

    let (config, digest) = sync::config()?;

    let list = config.convert_to_typed_array("sync")?;

    rpcenv["digest"] = proxmox::tools::digest_to_hex(&digest).into();

    Ok(list)
}

#[api(
    protected: true,
    input: {
        properties: {
            id: {
                schema: JOB_ID_SCHEMA,
            },
            store: {
                schema: DATASTORE_SCHEMA,
            },
            remote: {
                schema: REMOTE_ID_SCHEMA,
            },
            "remote-store": {
                schema: DATASTORE_SCHEMA,
            },
            "remove-vanished": {
                schema: REMOVE_VANISHED_BACKUPS_SCHEMA,
                optional: true,
            },
            comment: {
                optional: true,
                schema: SINGLE_LINE_COMMENT_SCHEMA,
            },
            schedule: {
                optional: true,
                schema: SYNC_SCHEDULE_SCHEMA,
            },
        },
    },
)]
/// Create a new sync job.
pub fn create_sync_job(param: Value) -> Result<(), Error> {

    let _lock = open_file_locked(sync::SYNC_CFG_LOCKFILE, std::time::Duration::new(10, 0), true)?;

    let sync_job: sync::SyncJobConfig = serde_json::from_value(param.clone())?;

    let (mut config, _digest) = sync::config()?;

    if let Some(_) = config.sections.get(&sync_job.id) {
        bail!("job '{}' already exists.", sync_job.id);
    }

    config.set_data(&sync_job.id, "sync", &sync_job)?;

    sync::save_config(&config)?;

    crate::server::jobstate::create_state_file("syncjob", &sync_job.id)?;

    Ok(())
}

#[api(
   input: {
        properties: {
            id: {
                schema: JOB_ID_SCHEMA,
            },
        },
    },
    returns: {
        description: "The sync job configuration.",
        type: sync::SyncJobConfig,
    },
)]
/// Read a sync job configuration.
pub fn read_sync_job(
    id: String,
    mut rpcenv: &mut dyn RpcEnvironment,
) -> Result<SyncJobConfig, Error> {
    let (config, digest) = sync::config()?;

    let sync_job = config.lookup("sync", &id)?;
    rpcenv["digest"] = proxmox::tools::digest_to_hex(&digest).into();

    Ok(sync_job)
}

#[api()]
#[derive(Serialize, Deserialize)]
#[serde(rename_all="kebab-case")]
#[allow(non_camel_case_types)]
/// Deletable property name
pub enum DeletableProperty {
    /// Delete the comment property.
    comment,
    /// Delete the job schedule.
    schedule,
    /// Delete the remove-vanished flag.
    remove_vanished,
}

#[api(
    protected: true,
    input: {
        properties: {
            id: {
                schema: JOB_ID_SCHEMA,
            },
            store: {
                schema: DATASTORE_SCHEMA,
                optional: true,
            },
            remote: {
                schema: REMOTE_ID_SCHEMA,
                optional: true,
            },
            "remote-store": {
                schema: DATASTORE_SCHEMA,
                optional: true,
            },
            "remove-vanished": {
                schema: REMOVE_VANISHED_BACKUPS_SCHEMA,
                optional: true,
            },
            comment: {
                optional: true,
                schema: SINGLE_LINE_COMMENT_SCHEMA,
            },
            schedule: {
                optional: true,
                schema: SYNC_SCHEDULE_SCHEMA,
            },
            delete: {
                description: "List of properties to delete.",
                type: Array,
                optional: true,
                items: {
                    type: DeletableProperty,
                }
            },
            digest: {
                optional: true,
                schema: PROXMOX_CONFIG_DIGEST_SCHEMA,
            },
        },
    },
)]
/// Update sync job config.
pub fn update_sync_job(
    id: String,
    store: Option<String>,
    remote: Option<String>,
    remote_store: Option<String>,
    remove_vanished: Option<bool>,
    comment: Option<String>,
    schedule: Option<String>,
    delete: Option<Vec<DeletableProperty>>,
    digest: Option<String>,
) -> Result<(), Error> {

    let _lock = open_file_locked(sync::SYNC_CFG_LOCKFILE, std::time::Duration::new(10, 0), true)?;

    // pass/compare digest
    let (mut config, expected_digest) = sync::config()?;

    if let Some(ref digest) = digest {
        let digest = proxmox::tools::hex_to_digest(digest)?;
        crate::tools::detect_modified_configuration_file(&digest, &expected_digest)?;
    }

    let mut data: sync::SyncJobConfig = config.lookup("sync", &id)?;

     if let Some(delete) = delete {
        for delete_prop in delete {
            match delete_prop {
                DeletableProperty::comment => { data.comment = None; },
                DeletableProperty::schedule => { data.schedule = None; },
                DeletableProperty::remove_vanished => { data.remove_vanished = None; },
            }
        }
    }

    if let Some(comment) = comment {
        let comment = comment.trim().to_string();
        if comment.is_empty() {
            data.comment = None;
        } else {
            data.comment = Some(comment);
        }
    }

    if let Some(store) = store { data.store = store; }
    if let Some(remote) = remote { data.remote = remote; }
    if let Some(remote_store) = remote_store { data.remote_store = remote_store; }
        
    
    if schedule.is_some() { data.schedule = schedule; }
    if remove_vanished.is_some() { data.remove_vanished = remove_vanished; }

    config.set_data(&id, "sync", &data)?;

    sync::save_config(&config)?;

    Ok(())
}

#[api(
    protected: true,
    input: {
        properties: {
            id: {
                schema: JOB_ID_SCHEMA,
            },
            digest: {
                optional: true,
                schema: PROXMOX_CONFIG_DIGEST_SCHEMA,
            },
        },
    },
)]
/// Remove a sync job configuration
pub fn delete_sync_job(id: String, digest: Option<String>) -> Result<(), Error> {

    let _lock = open_file_locked(sync::SYNC_CFG_LOCKFILE, std::time::Duration::new(10, 0), true)?;

    let (mut config, expected_digest) = sync::config()?;

    if let Some(ref digest) = digest {
        let digest = proxmox::tools::hex_to_digest(digest)?;
        crate::tools::detect_modified_configuration_file(&digest, &expected_digest)?;
    }

    match config.sections.get(&id) {
        Some(_) => { config.sections.remove(&id); },
        None => bail!("job '{}' does not exist.", id),
    }

    sync::save_config(&config)?;

    crate::server::jobstate::remove_state_file("syncjob", &id)?;

    Ok(())
}

const ITEM_ROUTER: Router = Router::new()
    .get(&API_METHOD_READ_SYNC_JOB)
    .put(&API_METHOD_UPDATE_SYNC_JOB)
    .delete(&API_METHOD_DELETE_SYNC_JOB);

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_LIST_SYNC_JOBS)
    .post(&API_METHOD_CREATE_SYNC_JOB)
    .match_all("id", &ITEM_ROUTER);
