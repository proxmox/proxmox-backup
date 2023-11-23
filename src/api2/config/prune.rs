use anyhow::Error;
use hex::FromHex;
use proxmox_sys::task_log;
use proxmox_sys::WorkerTaskContext;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use proxmox_router::{http_bail, Permission, Router, RpcEnvironment};
use proxmox_schema::{api, param_bail};

use pbs_api_types::{
    Authid, PruneJobConfig, PruneJobConfigUpdater, JOB_ID_SCHEMA, PRIV_DATASTORE_AUDIT,
    PRIV_DATASTORE_MODIFY, PROXMOX_CONFIG_DIGEST_SCHEMA,
};
use pbs_config::prune;

use pbs_config::CachedUserInfo;

#[api(
    input: {
        properties: {},
    },
    returns: {
        description: "List configured prune schedules.",
        type: Array,
        items: { type: PruneJobConfig },
    },
    access: {
        permission: &Permission::Anybody,
        // FIXME: Audit on namespaces
        description: "Requires Datastore.Audit.",
    },
)]
/// List all scheduled prune jobs.
pub fn list_prune_jobs(
    _param: Value,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<PruneJobConfig>, Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let user_info = CachedUserInfo::new()?;

    let required_privs = PRIV_DATASTORE_AUDIT | PRIV_DATASTORE_MODIFY;

    let (config, digest) = prune::config()?;

    let list = config.convert_to_typed_array("prune")?;

    let list = list
        .into_iter()
        .filter(|job: &PruneJobConfig| {
            let privs = user_info.lookup_privs(&auth_id, &job.acl_path());
            privs & required_privs != 00
        })
        .collect();

    rpcenv["digest"] = hex::encode(digest).into();

    Ok(list)
}

pub fn do_create_prune_job(
    config: PruneJobConfig,
    worker: Option<&dyn WorkerTaskContext>,
) -> Result<(), Error> {
    let _lock = prune::lock_config()?;

    let (mut section_config, _digest) = prune::config()?;

    if section_config.sections.get(&config.id).is_some() {
        param_bail!("id", "job '{}' already exists.", config.id);
    }

    section_config.set_data(&config.id, "prune", &config)?;

    prune::save_config(&section_config)?;

    crate::server::jobstate::create_state_file("prunejob", &config.id)?;

    if let Some(worker) = worker {
        task_log!(worker, "Prune job created: {}", config.id);
    }

    Ok(())
}

#[api(
    protected: true,
    input: {
        properties: {
            config: {
                type: PruneJobConfig,
                flatten: true,
            },
        },
    },
    access: {
        permission: &Permission::Anybody,
        description: "Requires Datastore.Modify on job's datastore.",
    },
)]
/// Create a new prune job.
pub fn create_prune_job(
    config: PruneJobConfig,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let user_info = CachedUserInfo::new()?;

    user_info.check_privs(&auth_id, &config.acl_path(), PRIV_DATASTORE_MODIFY, true)?;

    do_create_prune_job(config, None)
}

#[api(
   input: {
        properties: {
            id: {
                schema: JOB_ID_SCHEMA,
            },
        },
    },
    returns: { type: PruneJobConfig },
    access: {
        permission: &Permission::Anybody,
        description: "Requires Datastore.Audit or Datastore.Verify on job's datastore.",
    },
)]
/// Read a prune job configuration.
pub fn read_prune_job(
    id: String,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<PruneJobConfig, Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let user_info = CachedUserInfo::new()?;

    let (config, digest) = prune::config()?;

    let prune_job: PruneJobConfig = config.lookup("prune", &id)?;

    let required_privs = PRIV_DATASTORE_AUDIT;
    user_info.check_privs(&auth_id, &prune_job.acl_path(), required_privs, true)?;

    rpcenv["digest"] = hex::encode(digest).into();

    Ok(prune_job)
}

#[api]
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
/// Deletable property name
pub enum DeletableProperty {
    /// Delete the comment.
    Comment,
    /// Unset the disable flag.
    Disable,
    /// Reset the namespace to the root namespace.
    Ns,
    /// Reset the maximum depth to full recursion.
    MaxDepth,
    /// Delete number of last backups to keep.
    KeepLast,
    /// Delete number of hourly backups to keep.
    KeepHourly,
    /// Delete number of daily backups to keep.
    KeepDaily,
    /// Delete number of weekly backups to keep.
    KeepWeekly,
    /// Delete number of monthly backups to keep.
    KeepMonthly,
    /// Delete number of yearly backups to keep.
    KeepYearly,
}

#[api(
    protected: true,
    input: {
        properties: {
            id: {
                schema: JOB_ID_SCHEMA,
            },
            update: {
                type: PruneJobConfigUpdater,
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
        permission: &Permission::Anybody,
        description: "Requires Datastore.Modify on job's datastore.",
    },
)]
/// Update prune job config.
#[allow(clippy::too_many_arguments)]
pub fn update_prune_job(
    id: String,
    update: PruneJobConfigUpdater,
    delete: Option<Vec<DeletableProperty>>,
    digest: Option<String>,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let user_info = CachedUserInfo::new()?;

    let _lock = prune::lock_config()?;

    // pass/compare digest
    let (mut config, expected_digest) = prune::config()?;

    if let Some(ref digest) = digest {
        let digest = <[u8; 32]>::from_hex(digest)?;
        crate::tools::detect_modified_configuration_file(&digest, &expected_digest)?;
    }

    let mut data: PruneJobConfig = config.lookup("prune", &id)?;

    user_info.check_privs(&auth_id, &data.acl_path(), PRIV_DATASTORE_MODIFY, true)?;

    if let Some(delete) = delete {
        for delete_prop in delete {
            match delete_prop {
                DeletableProperty::Comment => {
                    data.comment = None;
                }
                DeletableProperty::Disable => {
                    data.disable = false;
                }
                DeletableProperty::Ns => {
                    data.options.ns = None;
                }
                DeletableProperty::MaxDepth => {
                    data.options.max_depth = None;
                }
                DeletableProperty::KeepLast => {
                    data.options.keep.keep_last = None;
                }
                DeletableProperty::KeepHourly => {
                    data.options.keep.keep_hourly = None;
                }
                DeletableProperty::KeepDaily => {
                    data.options.keep.keep_daily = None;
                }
                DeletableProperty::KeepWeekly => {
                    data.options.keep.keep_weekly = None;
                }
                DeletableProperty::KeepMonthly => {
                    data.options.keep.keep_monthly = None;
                }
                DeletableProperty::KeepYearly => {
                    data.options.keep.keep_yearly = None;
                }
            }
        }
    }

    let mut recheck_privs = false;
    if let Some(store) = update.store {
        // check new store with possibly new ns:
        recheck_privs = true;
        data.store = store;
    }

    if let Some(ns) = update.options.ns {
        recheck_privs = true;
        data.options.ns = if ns.is_root() { None } else { Some(ns) };
    }

    if recheck_privs {
        user_info.check_privs(&auth_id, &data.acl_path(), PRIV_DATASTORE_MODIFY, true)?;
    }

    let mut schedule_changed = false;
    if let Some(schedule) = update.schedule {
        schedule_changed = data.schedule != schedule;
        data.schedule = schedule;
    }

    if let Some(max_depth) = update.options.max_depth {
        if max_depth <= pbs_api_types::MAX_NAMESPACE_DEPTH {
            data.options.max_depth = Some(max_depth);
        }
    }

    if let Some(value) = update.disable {
        data.disable = value;
    }
    if let Some(value) = update.comment {
        data.comment = Some(value);
    }
    if let Some(value) = update.options.keep.keep_last {
        data.options.keep.keep_last = Some(value);
    }
    if let Some(value) = update.options.keep.keep_hourly {
        data.options.keep.keep_hourly = Some(value);
    }
    if let Some(value) = update.options.keep.keep_daily {
        data.options.keep.keep_daily = Some(value);
    }
    if let Some(value) = update.options.keep.keep_weekly {
        data.options.keep.keep_weekly = Some(value);
    }
    if let Some(value) = update.options.keep.keep_monthly {
        data.options.keep.keep_monthly = Some(value);
    }
    if let Some(value) = update.options.keep.keep_yearly {
        data.options.keep.keep_yearly = Some(value);
    }

    config.set_data(&id, "prune", &data)?;

    prune::save_config(&config)?;

    if schedule_changed {
        crate::server::jobstate::update_job_last_run_time("prunejob", &id)?;
    }

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
    access: {
        permission: &Permission::Anybody,
        description: "Requires Datastore.Verify on job's datastore.",
    },
)]
/// Remove a prune job configuration
pub fn delete_prune_job(
    id: String,
    digest: Option<String>,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let user_info = CachedUserInfo::new()?;

    let _lock = prune::lock_config()?;

    let (mut config, expected_digest) = prune::config()?;

    let job: PruneJobConfig = config.lookup("prune", &id)?;

    user_info.check_privs(&auth_id, &job.acl_path(), PRIV_DATASTORE_MODIFY, true)?;

    if let Some(ref digest) = digest {
        let digest = <[u8; 32]>::from_hex(digest)?;
        crate::tools::detect_modified_configuration_file(&digest, &expected_digest)?;
    }

    if config.sections.remove(&id).is_none() {
        http_bail!(NOT_FOUND, "job '{}' does not exist.", id);
    }

    prune::save_config(&config)?;

    crate::server::jobstate::remove_state_file("prunejob", &id)?;

    Ok(())
}

const ITEM_ROUTER: Router = Router::new()
    .get(&API_METHOD_READ_PRUNE_JOB)
    .put(&API_METHOD_UPDATE_PRUNE_JOB)
    .delete(&API_METHOD_DELETE_PRUNE_JOB);

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_LIST_PRUNE_JOBS)
    .post(&API_METHOD_CREATE_PRUNE_JOB)
    .match_all("id", &ITEM_ROUTER);
