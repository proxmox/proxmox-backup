use std::path::PathBuf;

use anyhow::{bail, Error};
use serde_json::Value;
use ::serde::{Deserialize, Serialize};

use proxmox::api::{api, Router, RpcEnvironment, Permission};
use proxmox::api::section_config::SectionConfigData;
use proxmox::api::schema::parse_property_string;

use pbs_datastore::task::TaskState;

use crate::api2::config::sync::delete_sync_job;
use crate::api2::config::verify::delete_verification_job;
use crate::api2::config::tape_backup_job::{list_tape_backup_jobs, delete_tape_backup_job};
use crate::api2::admin::{
    sync::list_sync_jobs,
    verify::list_verification_jobs,
};
use crate::api2::types::*;
use crate::backup::*;
use crate::config::cached_user_info::CachedUserInfo;
use crate::config::datastore::{self, DataStoreConfig, DIR_NAME_SCHEMA};
use crate::config::acl::{PRIV_DATASTORE_ALLOCATE, PRIV_DATASTORE_AUDIT, PRIV_DATASTORE_MODIFY};
use crate::server::{jobstate, WorkerTask};

#[api(
    input: {
        properties: {},
    },
    returns: {
        description: "List the configured datastores (with config digest).",
        type: Array,
        items: { type: datastore::DataStoreConfig },
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

    let (config, digest) = datastore::config()?;

    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let user_info = CachedUserInfo::new()?;

    rpcenv["digest"] = proxmox::tools::digest_to_hex(&digest).into();

    let list:Vec<DataStoreConfig> = config.convert_to_typed_array("datastore")?;
    let filter_by_privs = |store: &DataStoreConfig| {
        let user_privs = user_info.lookup_privs(&auth_id, &["datastore", &store.name]);
        (user_privs & PRIV_DATASTORE_AUDIT) != 0
    };

    Ok(list.into_iter().filter(filter_by_privs).collect())
}

pub(crate) fn do_create_datastore(
    _lock: std::fs::File,
    mut config: SectionConfigData,
    datastore: DataStoreConfig,
    worker: Option<&dyn TaskState>,
) -> Result<(), Error> {
    let path: PathBuf = datastore.path.clone().into();

    let backup_user = crate::backup::backup_user()?;
    let _store = ChunkStore::create(&datastore.name, path, backup_user.uid, backup_user.gid, worker)?;

    config.set_data(&datastore.name, "datastore", &datastore)?;

    datastore::save_config(&config)?;

    jobstate::create_state_file("prune", &datastore.name)?;
    jobstate::create_state_file("garbage_collection", &datastore.name)?;

    Ok(())
}

// fixme: impl. const fn get_object_schema(datastore::DataStoreConfig::API_SCHEMA),
// but this need support for match inside const fn
// see: https://github.com/rust-lang/rust/issues/49146

#[api(
    protected: true,
    input: {
        properties: {
            name: {
                schema: DATASTORE_SCHEMA,
            },
            path: {
                schema: DIR_NAME_SCHEMA,
            },
            comment: {
                optional: true,
                schema: SINGLE_LINE_COMMENT_SCHEMA,
            },
            "notify-user": {
                optional: true,
                type: Userid,
            },
            "notify": {
                optional: true,
                schema: DATASTORE_NOTIFY_STRING_SCHEMA,
            },
            "gc-schedule": {
                optional: true,
                schema: GC_SCHEDULE_SCHEMA,
            },
            "prune-schedule": {
                optional: true,
                schema: PRUNE_SCHEDULE_SCHEMA,
            },
            "keep-last": {
                optional: true,
                schema: PRUNE_SCHEMA_KEEP_LAST,
            },
            "keep-hourly": {
                optional: true,
                schema: PRUNE_SCHEMA_KEEP_HOURLY,
            },
            "keep-daily": {
                optional: true,
                schema: PRUNE_SCHEMA_KEEP_DAILY,
            },
            "keep-weekly": {
                optional: true,
                schema: PRUNE_SCHEMA_KEEP_WEEKLY,
            },
            "keep-monthly": {
                optional: true,
                schema: PRUNE_SCHEMA_KEEP_MONTHLY,
            },
            "keep-yearly": {
                optional: true,
                schema: PRUNE_SCHEMA_KEEP_YEARLY,
            },
        },
    },
    access: {
        permission: &Permission::Privilege(&["datastore"], PRIV_DATASTORE_ALLOCATE, false),
    },
)]
/// Create new datastore config.
pub fn create_datastore(
    param: Value,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<String, Error> {

    let lock = datastore::lock_config()?;

    let datastore: datastore::DataStoreConfig = serde_json::from_value(param)?;

    let (config, _digest) = datastore::config()?;

    if config.sections.get(&datastore.name).is_some() {
        bail!("datastore '{}' already exists.", datastore.name);
    }

    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;

    WorkerTask::new_thread(
        "create-datastore",
        Some(datastore.name.to_string()),
        auth_id,
        false,
        move |worker| do_create_datastore(lock, config, datastore, Some(&worker)),
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
    returns: { type: datastore::DataStoreConfig },
    access: {
        permission: &Permission::Privilege(&["datastore", "{name}"], PRIV_DATASTORE_AUDIT, false),
    },
)]
/// Read a datastore configuration.
pub fn read_datastore(
    name: String,
    mut rpcenv: &mut dyn RpcEnvironment,
) -> Result<DataStoreConfig, Error> {
    let (config, digest) = datastore::config()?;

    let store_config = config.lookup("datastore", &name)?;
    rpcenv["digest"] = proxmox::tools::digest_to_hex(&digest).into();

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
}

#[api(
    protected: true,
    input: {
        properties: {
            name: {
                schema: DATASTORE_SCHEMA,
            },
            comment: {
                optional: true,
                schema: SINGLE_LINE_COMMENT_SCHEMA,
            },
            "notify-user": {
                optional: true,
                type: Userid,
            },
            "notify": {
                optional: true,
                schema: DATASTORE_NOTIFY_STRING_SCHEMA,
            },
            "gc-schedule": {
                optional: true,
                schema: GC_SCHEDULE_SCHEMA,
            },
            "prune-schedule": {
                optional: true,
                schema: PRUNE_SCHEDULE_SCHEMA,
            },
            "keep-last": {
                optional: true,
                schema: PRUNE_SCHEMA_KEEP_LAST,
            },
            "keep-hourly": {
                optional: true,
                schema: PRUNE_SCHEMA_KEEP_HOURLY,
            },
            "keep-daily": {
                optional: true,
                schema: PRUNE_SCHEMA_KEEP_DAILY,
            },
            "keep-weekly": {
                optional: true,
                schema: PRUNE_SCHEMA_KEEP_WEEKLY,
            },
            "keep-monthly": {
                optional: true,
                schema: PRUNE_SCHEMA_KEEP_MONTHLY,
            },
            "keep-yearly": {
                optional: true,
                schema: PRUNE_SCHEMA_KEEP_YEARLY,
            },
            "verify-new": {
                description: "If enabled, all new backups will be verified right after completion.",
                type: bool,
                optional: true,
                default: false,
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
#[allow(clippy::too_many_arguments)]
pub fn update_datastore(
    name: String,
    comment: Option<String>,
    gc_schedule: Option<String>,
    prune_schedule: Option<String>,
    keep_last: Option<u64>,
    keep_hourly: Option<u64>,
    keep_daily: Option<u64>,
    keep_weekly: Option<u64>,
    keep_monthly: Option<u64>,
    keep_yearly: Option<u64>,
    verify_new: Option<bool>,
    notify: Option<String>,
    notify_user: Option<Userid>,
    delete: Option<Vec<DeletableProperty>>,
    digest: Option<String>,
) -> Result<(), Error> {

    let _lock = datastore::lock_config()?;

    // pass/compare digest
    let (mut config, expected_digest) = datastore::config()?;

    if let Some(ref digest) = digest {
        let digest = proxmox::tools::hex_to_digest(digest)?;
        crate::tools::detect_modified_configuration_file(&digest, &expected_digest)?;
    }

    let mut data: datastore::DataStoreConfig = config.lookup("datastore", &name)?;

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

    let mut gc_schedule_changed = false;
    if gc_schedule.is_some() {
        gc_schedule_changed = data.gc_schedule != gc_schedule;
        data.gc_schedule = gc_schedule;
    }

    let mut prune_schedule_changed = false;
    if prune_schedule.is_some() {
        prune_schedule_changed = data.prune_schedule != prune_schedule;
        data.prune_schedule = prune_schedule;
    }

    if keep_last.is_some() { data.keep_last = keep_last; }
    if keep_hourly.is_some() { data.keep_hourly = keep_hourly; }
    if keep_daily.is_some() { data.keep_daily = keep_daily; }
    if keep_weekly.is_some() { data.keep_weekly = keep_weekly; }
    if keep_monthly.is_some() { data.keep_monthly = keep_monthly; }
    if keep_yearly.is_some() { data.keep_yearly = keep_yearly; }

    if let Some(notify_str) = notify {
        let value = parse_property_string(&notify_str, &DatastoreNotify::API_SCHEMA)?;
        let notify: DatastoreNotify = serde_json::from_value(value)?;
        if let  DatastoreNotify { gc: None, verify: None, sync: None } = notify {
            data.notify = None;
        } else {
            data.notify = Some(notify_str);
        }
    }
    if verify_new.is_some() { data.verify_new = verify_new; }

    if notify_user.is_some() { data.notify_user = notify_user; }

    config.set_data(&name, "datastore", &data)?;

    datastore::save_config(&config)?;

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

    let _lock = datastore::lock_config()?;

    let (mut config, expected_digest) = datastore::config()?;

    if let Some(ref digest) = digest {
        let digest = proxmox::tools::hex_to_digest(digest)?;
        crate::tools::detect_modified_configuration_file(&digest, &expected_digest)?;
    }

    match config.sections.get(&name) {
        Some(_) => { config.sections.remove(&name); },
        None => bail!("datastore '{}' does not exist.", name),
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

    datastore::save_config(&config)?;

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
