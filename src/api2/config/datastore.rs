use std::path::PathBuf;

use ::serde::{Deserialize, Serialize};
use anyhow::Error;
use hex::FromHex;
use serde_json::Value;

use proxmox_router::{http_bail, Permission, Router, RpcEnvironment, RpcEnvironmentType};
use proxmox_schema::{api, param_bail, ApiType};
use proxmox_section_config::SectionConfigData;
use proxmox_sys::{task_warn, WorkerTaskContext};
use proxmox_uuid::Uuid;

use pbs_api_types::{
    Authid, DataStoreConfig, DataStoreConfigUpdater, DatastoreNotify, DatastoreTuning, KeepOptions,
    PruneJobConfig, PruneJobOptions, DATASTORE_SCHEMA, PRIV_DATASTORE_ALLOCATE,
    PRIV_DATASTORE_AUDIT, PRIV_DATASTORE_MODIFY, PROXMOX_CONFIG_DIGEST_SCHEMA, UPID_SCHEMA,
};
use pbs_config::BackupLockGuard;
use pbs_datastore::chunk_store::ChunkStore;

use crate::api2::admin::{
    prune::list_prune_jobs, sync::list_sync_jobs, verify::list_verification_jobs,
};
use crate::api2::config::prune::{delete_prune_job, do_create_prune_job};
use crate::api2::config::sync::delete_sync_job;
use crate::api2::config::tape_backup_job::{delete_tape_backup_job, list_tape_backup_jobs};
use crate::api2::config::verify::delete_verification_job;
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
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<DataStoreConfig>, Error> {
    let (config, digest) = pbs_config::datastore::config()?;

    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let user_info = CachedUserInfo::new()?;

    rpcenv["digest"] = hex::encode(digest).into();

    let list: Vec<DataStoreConfig> = config.convert_to_typed_array("datastore")?;
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

    let tuning: DatastoreTuning = serde_json::from_value(
        DatastoreTuning::API_SCHEMA
            .parse_property_string(datastore.tuning.as_deref().unwrap_or(""))?,
    )?;
    let backup_user = pbs_config::backup_user()?;
    let _store = ChunkStore::create(
        &datastore.name,
        path,
        backup_user.uid,
        backup_user.gid,
        worker,
        tuning.sync_level.unwrap_or_default(),
    )?;

    config.set_data(&datastore.name, "datastore", &datastore)?;

    pbs_config::datastore::save_config(&config)?;

    jobstate::create_state_file("garbage_collection", &datastore.name)
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

    let prune_job_config = config.prune_schedule.as_ref().map(|schedule| {
        let mut id = format!("default-{}-{}", config.name, Uuid::generate());
        id.truncate(32);

        PruneJobConfig {
            id,
            store: config.name.clone(),
            comment: None,
            disable: false,
            schedule: schedule.clone(),
            options: PruneJobOptions {
                keep: config.keep.clone(),
                max_depth: None,
                ns: None,
            },
        }
    });

    // clearing prune settings in the datastore config, as they are now handled by prune jobs
    let config = DataStoreConfig {
        prune_schedule: None,
        keep: KeepOptions::default(),
        ..config
    };

    WorkerTask::new_thread(
        "create-datastore",
        Some(config.name.to_string()),
        auth_id.to_string(),
        to_stdout,
        move |worker| {
            do_create_datastore(lock, section_config, config, Some(&worker))?;

            if let Some(prune_job_config) = prune_job_config {
                do_create_prune_job(prune_job_config, Some(&worker))
            } else {
                Ok(())
            }
        },
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
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<DataStoreConfig, Error> {
    let (config, digest) = pbs_config::datastore::config()?;

    let store_config = config.lookup("datastore", &name)?;
    rpcenv["digest"] = hex::encode(digest).into();

    Ok(store_config)
}

#[api()]
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
/// Deletable property name
pub enum DeletableProperty {
    /// Delete the comment property.
    Comment,
    /// Delete the garbage collection schedule.
    GcSchedule,
    /// Delete the prune job schedule.
    PruneSchedule,
    /// Delete the keep-last property
    KeepLast,
    /// Delete the keep-hourly property
    KeepHourly,
    /// Delete the keep-daily property
    KeepDaily,
    /// Delete the keep-weekly property
    KeepWeekly,
    /// Delete the keep-monthly property
    KeepMonthly,
    /// Delete the keep-yearly property
    KeepYearly,
    /// Delete the verify-new property
    VerifyNew,
    /// Delete the notify-user property
    NotifyUser,
    /// Delete the notify property
    Notify,
    /// Delete the tuning property
    Tuning,
    /// Delete the maintenance-mode property
    MaintenanceMode,
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
                DeletableProperty::Comment => {
                    data.comment = None;
                }
                DeletableProperty::GcSchedule => {
                    data.gc_schedule = None;
                }
                DeletableProperty::PruneSchedule => {
                    data.prune_schedule = None;
                }
                DeletableProperty::KeepLast => {
                    data.keep.keep_last = None;
                }
                DeletableProperty::KeepHourly => {
                    data.keep.keep_hourly = None;
                }
                DeletableProperty::KeepDaily => {
                    data.keep.keep_daily = None;
                }
                DeletableProperty::KeepWeekly => {
                    data.keep.keep_weekly = None;
                }
                DeletableProperty::KeepMonthly => {
                    data.keep.keep_monthly = None;
                }
                DeletableProperty::KeepYearly => {
                    data.keep.keep_yearly = None;
                }
                DeletableProperty::VerifyNew => {
                    data.verify_new = None;
                }
                DeletableProperty::Notify => {
                    data.notify = None;
                }
                DeletableProperty::NotifyUser => {
                    data.notify_user = None;
                }
                DeletableProperty::Tuning => {
                    data.tuning = None;
                }
                DeletableProperty::MaintenanceMode => {
                    data.maintenance_mode = None;
                }
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

    macro_rules! prune_disabled {
        ($(($param:literal, $($member:tt)+)),+) => {
            $(
                if update.$($member)+.is_some() {
                    param_bail!(
                        $param,
                        "datastore prune settings have been replaced by prune jobs",
                    );
                }
            )+
        };
    }
    prune_disabled! {
        ("keep-last", keep.keep_last),
        ("keep-hourly", keep.keep_hourly),
        ("keep-daily", keep.keep_daily),
        ("keep-weekly", keep.keep_weekly),
        ("keep-monthly", keep.keep_monthly),
        ("keep-yearly", keep.keep_yearly),
        ("prune-schedule", prune_schedule)
    }

    if let Some(notify_str) = update.notify {
        let value = DatastoreNotify::API_SCHEMA.parse_property_string(&notify_str)?;
        let notify: DatastoreNotify = serde_json::from_value(value)?;
        if let DatastoreNotify {
            gc: None,
            verify: None,
            sync: None,
            prune: None,
        } = notify
        {
            data.notify = None;
        } else {
            data.notify = Some(notify_str);
        }
    }
    if update.verify_new.is_some() {
        data.verify_new = update.verify_new;
    }

    if update.notify_user.is_some() {
        data.notify_user = update.notify_user;
    }

    if update.tuning.is_some() {
        data.tuning = update.tuning;
    }

    if update.maintenance_mode.is_some() {
        data.maintenance_mode = update.maintenance_mode;
    }

    config.set_data(&name, "datastore", &data)?;

    pbs_config::datastore::save_config(&config)?;

    // we want to reset the statefiles, to avoid an immediate action in some cases
    // (e.g. going from monthly to weekly in the second week of the month)
    if gc_schedule_changed {
        jobstate::update_job_last_run_time("garbage_collection", &name)?;
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
            "destroy-data": {
                description: "Delete the datastore's underlying contents",
                optional: true,
                type: bool,
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
    returns: {
        schema: UPID_SCHEMA,
    },
)]
/// Remove a datastore configuration and optionally delete all its contents.
pub async fn delete_datastore(
    name: String,
    keep_job_configs: bool,
    destroy_data: bool,
    digest: Option<String>,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<String, Error> {
    let _lock = pbs_config::datastore::lock_config()?;

    let (config, expected_digest) = pbs_config::datastore::config()?;

    if let Some(ref digest) = digest {
        let digest = <[u8; 32]>::from_hex(digest)?;
        crate::tools::detect_modified_configuration_file(&digest, &expected_digest)?;
    }

    if !config.sections.contains_key(&name) {
        http_bail!(NOT_FOUND, "datastore '{}' does not exist.", name);
    }

    if !keep_job_configs {
        for job in list_verification_jobs(Some(name.clone()), Value::Null, rpcenv)? {
            delete_verification_job(job.config.id, None, rpcenv)?
        }
        for job in list_sync_jobs(Some(name.clone()), Value::Null, rpcenv)? {
            delete_sync_job(job.config.id, None, rpcenv)?
        }
        for job in list_prune_jobs(Some(name.clone()), Value::Null, rpcenv)? {
            delete_prune_job(job.config.id, None, rpcenv)?
        }

        let (mut tree, _digest) = pbs_config::acl::config()?;
        tree.delete_node(&format!("/datastore/{}", name));
        pbs_config::acl::save_config(&tree)?;

        let tape_jobs = list_tape_backup_jobs(Value::Null, rpcenv)?;
        for job_config in tape_jobs
            .into_iter()
            .filter(|config| config.setup.store == name)
        {
            delete_tape_backup_job(job_config.id, None, rpcenv)?;
        }
    }

    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let to_stdout = rpcenv.env_type() == RpcEnvironmentType::CLI;

    let upid = WorkerTask::new_thread(
        "delete-datastore",
        Some(name.clone()),
        auth_id.to_string(),
        to_stdout,
        move |worker| {
            pbs_datastore::DataStore::destroy(&name, destroy_data, &worker)?;

            // ignore errors
            let _ = jobstate::remove_state_file("prune", &name);
            let _ = jobstate::remove_state_file("garbage_collection", &name);

            if let Err(err) =
                proxmox_async::runtime::block_on(crate::server::notify_datastore_removed())
            {
                task_warn!(worker, "failed to notify after datastore removal: {err}");
            }

            Ok(())
        },
    )?;

    Ok(upid)
}

const ITEM_ROUTER: Router = Router::new()
    .get(&API_METHOD_READ_DATASTORE)
    .put(&API_METHOD_UPDATE_DATASTORE)
    .delete(&API_METHOD_DELETE_DATASTORE);

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_LIST_DATASTORES)
    .post(&API_METHOD_CREATE_DATASTORE)
    .match_all("name", &ITEM_ROUTER);
