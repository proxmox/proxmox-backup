use std::path::PathBuf;

use anyhow::Error;
use serde_json::Value;
use ::serde::{Deserialize, Serialize};
use hex::FromHex;

use proxmox_router::{http_bail, Router, RpcEnvironment, RpcEnvironmentType, Permission};
use proxmox_schema::{api, param_bail, ApiType};
use proxmox_section_config::SectionConfigData;
use proxmox_sys::WorkerTaskContext;

use pbs_datastore::chunk_store::ChunkStore;
use pbs_config::BackupLockGuard;
use pbs_api_types::{
    Authid, DatastoreNotify,
    DATASTORE_SCHEMA, PROXMOX_CONFIG_DIGEST_SCHEMA,
    PRIV_DATASTORE_ALLOCATE, PRIV_DATASTORE_AUDIT, PRIV_DATASTORE_MODIFY,
    DataStoreConfig, DataStoreConfigUpdater,
};

use crate::api2::config::sync::delete_sync_job;
use crate::api2::config::verify::delete_verification_job;
use crate::api2::config::tape_backup_job::{list_tape_backup_jobs, delete_tape_backup_job};
use crate::api2::admin::{
    sync::list_sync_jobs,
    verify::list_verification_jobs,
};
use pbs_config::CachedUserInfo;

use proxmox_rest_server::WorkerTask;

use crate::server::jobstate;

#[api(
    input: {
        properties: {},
    },
    returns: {
        description: "List the configured datastores (with config digest).",
        type: Array,
        items: { type: DataStoreConfig },
    },
    access: {
        permission: &Permission::Anybody,
    },
)]
/// List all datastores
pub fn list_datastores(
    _param: Value,
    mut rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<DataStoreConfig>, Error> {

    let (config, digest) = pbs_config::datastore::config()?;

    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let user_info = CachedUserInfo::new()?;

    rpcenv["digest"] = hex::encode(&digest).into();

    let list:Vec<DataStoreConfig> = config.convert_to_typed_array("datastore")?;
    let filter_by_privs = |store: &DataStoreConfig| {
        let user_privs = user_info.lookup_privs(&auth_id, &["datastore", &store.name]);
        (user_privs & PRIV_DATASTORE_AUDIT) != 0
    };

    Ok(list.into_iter().filter(filter_by_privs).collect())
}

pub(crate) fn do_create_datastore(
    _lock: BackupLockGuard,
    mut config: SectionConfigData,
    datastore: DataStoreConfig,
    worker: Option<&dyn WorkerTaskContext>,
) -> Result<(), Error> {
    let path: PathBuf = datastore.path.clone().into();

    let backup_user = pbs_config::backup_user()?;
    let _store = ChunkStore::create(&datastore.name, path, backup_user.uid, backup_user.gid, worker)?;

    config.set_data(&datastore.name, "datastore", &datastore)?;

    pbs_config::datastore::save_config(&config)?;

    jobstate::create_state_file("prune", &datastore.name)?;
    jobstate::create_state_file("garbage_collection", &datastore.name)?;

    Ok(())
}

#[api(
    protected: true,
    input: {
        properties: {
            config: {
                type: DataStoreConfig,
                flatten: true,
            },
        },
    },
    access: {
        permission: &Permission::Privilege(&["datastore"], PRIV_DATASTORE_ALLOCATE, false),
    },
)]
/// Create new datastore config.
pub fn create_datastore(
    config: DataStoreConfig,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<String, Error> {

    let lock = pbs_config::datastore::lock_config()?;

    let (section_config, _digest) = pbs_config::datastore::config()?;

    if section_config.sections.get(&config.name).is_some() {
        param_bail!("name", "datastore '{}' already exists.", config.name);
    }

    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let to_stdout = rpcenv.env_type() == RpcEnvironmentType::CLI;

    WorkerTask::new_thread(
        "create-datastore",
        Some(config.name.to_string()),
        auth_id.to_string(),
        to_stdout,
       move |worker| do_create_datastore(lock, section_config, config, Some(&worker)),
    )
}

#[api(
   input: {
        properties: {
            name: {
                schema: DATASTORE_SCHEMA,
            },
        },
    },
    returns: { type: DataStoreConfig },
    access: {
        permission: &Permission::Privilege(&["datastore", "{name}"], PRIV_DATASTORE_AUDIT, false),
    },
)]
/// Read a datastore configuration.
pub fn read_datastore(
    name: String,
    mut rpcenv: &mut dyn RpcEnvironment,
) -> Result<DataStoreConfig, Error> {
    let (config, digest) = pbs_config::datastore::config()?;

    let store_config = config.lookup("datastore", &name)?;
    rpcenv["digest"] = hex::encode(&digest).into();

    Ok(store_config)
}

#[api()]
#[derive(Serialize, Deserialize)]
#[serde(rename_all="kebab-case")]
#[allow(non_camel_case_types)]
/// Deletable property name
pub enum DeletableProperty {
    /// Delete the comment property.
    comment,
    /// Delete the garbage collection schedule.
    gc_schedule,
    /// Delete the prune job schedule.
    prune_schedule,
    /// Delete the keep-last property
    keep_last,
    /// Delete the keep-hourly property
    keep_hourly,
    /// Delete the keep-daily property
    keep_daily,
    /// Delete the keep-weekly property
    keep_weekly,
    /// Delete the keep-monthly property
    keep_monthly,
    /// Delete the keep-yearly property
    keep_yearly,
    /// Delete the verify-new property
    verify_new,
    /// Delete the notify-user property
    notify_user,
    /// Delete the notify property
    notify,
    /// Delete the tuning property
    tuning,
}

#[api(
    protected: true,
    input: {
        properties: {
            name: {
                schema: DATASTORE_SCHEMA,
            },
            update: {
                type: DataStoreConfigUpdater,
                flatten: true,
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
    access: {
        permission: &Permission::Privilege(&["datastore", "{name}"], PRIV_DATASTORE_MODIFY, false),
    },
)]
/// Update datastore config.
pub fn update_datastore(
    update: DataStoreConfigUpdater,
    name: String,
    delete: Option<Vec<DeletableProperty>>,
    digest: Option<String>,
) -> Result<(), Error> {

    let _lock = pbs_config::datastore::lock_config()?;

    // pass/compare digest
    let (mut config, expected_digest) = pbs_config::datastore::config()?;

    if let Some(ref digest) = digest {
        let digest = <[u8; 32]>::from_hex(digest)?;
        crate::tools::detect_modified_configuration_file(&digest, &expected_digest)?;
    }

    let mut data: DataStoreConfig = config.lookup("datastore", &name)?;

     if let Some(delete) = delete {
        for delete_prop in delete {
            match delete_prop {
                DeletableProperty::comment => { data.comment = None; },
                DeletableProperty::gc_schedule => { data.gc_schedule = None; },
                DeletableProperty::prune_schedule => { data.prune_schedule = None; },
                DeletableProperty::keep_last => { data.keep_last = None; },
                DeletableProperty::keep_hourly => { data.keep_hourly = None; },
                DeletableProperty::keep_daily => { data.keep_daily = None; },
                DeletableProperty::keep_weekly => { data.keep_weekly = None; },
                DeletableProperty::keep_monthly => { data.keep_monthly = None; },
                DeletableProperty::keep_yearly => { data.keep_yearly = None; },
                DeletableProperty::verify_new => { data.verify_new = None; },
                DeletableProperty::notify => { data.notify = None; },
                DeletableProperty::notify_user => { data.notify_user = None; },
                DeletableProperty::tuning => { data.tuning = None; },
            }
        }
    }

    if let Some(comment) = update.comment {
        let comment = comment.trim().to_string();
        if comment.is_empty() {
            data.comment = None;
        } else {
            data.comment = Some(comment);
        }
    }

    let mut gc_schedule_changed = false;
    if update.gc_schedule.is_some() {
        gc_schedule_changed = data.gc_schedule != update.gc_schedule;
        data.gc_schedule = update.gc_schedule;
    }

    let mut prune_schedule_changed = false;
    if update.prune_schedule.is_some() {
        prune_schedule_changed = data.prune_schedule != update.prune_schedule;
        data.prune_schedule = update.prune_schedule;
    }

    if update.keep_last.is_some() { data.keep_last = update.keep_last; }
    if update.keep_hourly.is_some() { data.keep_hourly = update.keep_hourly; }
    if update.keep_daily.is_some() { data.keep_daily = update.keep_daily; }
    if update.keep_weekly.is_some() { data.keep_weekly = update.keep_weekly; }
    if update.keep_monthly.is_some() { data.keep_monthly = update.keep_monthly; }
    if update.keep_yearly.is_some() { data.keep_yearly = update.keep_yearly; }

    if let Some(notify_str) = update.notify {
        let value = DatastoreNotify::API_SCHEMA.parse_property_string(&notify_str)?;
        let notify: DatastoreNotify = serde_json::from_value(value)?;
        if let  DatastoreNotify { gc: None, verify: None, sync: None } = notify {
            data.notify = None;
        } else {
            data.notify = Some(notify_str);
        }
    }
    if update.verify_new.is_some() { data.verify_new = update.verify_new; }

    if update.notify_user.is_some() { data.notify_user = update.notify_user; }

    if update.tuning.is_some() { data.tuning = update.tuning; }

    config.set_data(&name, "datastore", &data)?;

    pbs_config::datastore::save_config(&config)?;

    // we want to reset the statefiles, to avoid an immediate action in some cases
    // (e.g. going from monthly to weekly in the second week of the month)
    if gc_schedule_changed {
        jobstate::update_job_last_run_time("garbage_collection", &name)?;
    }

    if prune_schedule_changed {
        jobstate::update_job_last_run_time("prune", &name)?;
    }

    Ok(())
}

#[api(
    protected: true,
    input: {
        properties: {
            name: {
                schema: DATASTORE_SCHEMA,
            },
            "keep-job-configs": {
                description: "If enabled, the job configurations related to this datastore will be kept.",
                type: bool,
                optional: true,
                default: false,
            },
            digest: {
                optional: true,
                schema: PROXMOX_CONFIG_DIGEST_SCHEMA,
            },
        },
    },
    access: {
        permission: &Permission::Privilege(&["datastore", "{name}"], PRIV_DATASTORE_ALLOCATE, false),
    },
)]
/// Remove a datastore configuration.
pub async fn delete_datastore(
    name: String,
    keep_job_configs: bool,
    digest: Option<String>,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {

    let _lock = pbs_config::datastore::lock_config()?;

    let (mut config, expected_digest) = pbs_config::datastore::config()?;

    if let Some(ref digest) = digest {
        let digest = <[u8; 32]>::from_hex(digest)?;
        crate::tools::detect_modified_configuration_file(&digest, &expected_digest)?;
    }

    match config.sections.get(&name) {
        Some(_) => { config.sections.remove(&name); },
        None => http_bail!(NOT_FOUND, "datastore '{}' does not exist.", name),
    }

    if !keep_job_configs {
        for job in list_verification_jobs(Some(name.clone()), Value::Null, rpcenv)? {
            delete_verification_job(job.config.id, None, rpcenv)?
        }
        for job in list_sync_jobs(Some(name.clone()), Value::Null, rpcenv)? {
            delete_sync_job(job.config.id, None, rpcenv)?
        }

        let tape_jobs = list_tape_backup_jobs(Value::Null, rpcenv)?;
        for job_config in  tape_jobs.into_iter().filter(|config| config.setup.store == name) {
            delete_tape_backup_job(job_config.id, None, rpcenv)?;
        }
    }

    pbs_config::datastore::save_config(&config)?;

    // ignore errors
    let _ = jobstate::remove_state_file("prune", &name);
    let _ = jobstate::remove_state_file("garbage_collection", &name);

    crate::server::notify_datastore_removed().await?;

    Ok(())
}

const ITEM_ROUTER: Router = Router::new()
    .get(&API_METHOD_READ_DATASTORE)
    .put(&API_METHOD_UPDATE_DATASTORE)
    .delete(&API_METHOD_DELETE_DATASTORE);

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_LIST_DATASTORES)
    .post(&API_METHOD_CREATE_DATASTORE)
    .match_all("name", &ITEM_ROUTER);
