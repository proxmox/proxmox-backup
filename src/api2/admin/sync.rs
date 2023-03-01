//! Datastore Synchronization Job Management

use anyhow::{bail, format_err, Error};
use serde_json::Value;

use proxmox_router::{
    list_subdirs_api_method, ApiMethod, Permission, Router, RpcEnvironment, RpcEnvironmentType,
    SubdirMap,
};
use proxmox_schema::api;
use proxmox_sortable_macro::sortable;

use pbs_api_types::{Authid, SyncJobConfig, SyncJobStatus, DATASTORE_SCHEMA, JOB_ID_SCHEMA};
use pbs_config::sync;
use pbs_config::CachedUserInfo;

use crate::{
    api2::{
        config::sync::{check_sync_job_modify_access, check_sync_job_read_access},
        pull::do_sync_job,
    },
    server::jobstate::{compute_schedule_status, Job, JobState},
};

#[api(
    input: {
        properties: {
            store: {
                schema: DATASTORE_SCHEMA,
                optional: true,
            },
        },
    },
    returns: {
        description: "List configured jobs and their status.",
        type: Array,
        items: { type: SyncJobStatus },
    },
    access: {
        description: "Limited to sync jobs where user has Datastore.Audit on target datastore, and Remote.Audit on source remote.",
        permission: &Permission::Anybody,
    },
)]
/// List all sync jobs
pub fn list_sync_jobs(
    store: Option<String>,
    _param: Value,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<SyncJobStatus>, Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let user_info = CachedUserInfo::new()?;

    let (config, digest) = sync::config()?;

    let job_config_iter = config
        .convert_to_typed_array("sync")?
        .into_iter()
        .filter(|job: &SyncJobConfig| {
            if let Some(store) = &store {
                &job.store == store
            } else {
                true
            }
        })
        .filter(|job: &SyncJobConfig| check_sync_job_read_access(&user_info, &auth_id, job));

    let mut list = Vec::new();

    for job in job_config_iter {
        let last_state = JobState::load("syncjob", &job.id)
            .map_err(|err| format_err!("could not open statefile for {}: {}", &job.id, err))?;

        let status = compute_schedule_status(&last_state, job.schedule.as_deref())?;

        list.push(SyncJobStatus {
            config: job,
            status,
        });
    }

    rpcenv["digest"] = hex::encode(digest).into();

    Ok(list)
}

#[api(
    input: {
        properties: {
            id: {
                schema: JOB_ID_SCHEMA,
            }
        }
    },
    access: {
        description: "User needs Datastore.Backup on target datastore, and Remote.Read on source remote. Additionally, remove_vanished requires Datastore.Prune, and any owner other than the user themselves requires Datastore.Modify",
        permission: &Permission::Anybody,
    },
)]
/// Runs the sync jobs manually.
pub fn run_sync_job(
    id: String,
    _info: &ApiMethod,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<String, Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let user_info = CachedUserInfo::new()?;

    let (config, _digest) = sync::config()?;
    let sync_job: SyncJobConfig = config.lookup("sync", &id)?;

    if !check_sync_job_modify_access(&user_info, &auth_id, &sync_job) {
        bail!("permission check failed");
    }

    let job = Job::new("syncjob", &id)?;

    let to_stdout = rpcenv.env_type() == RpcEnvironmentType::CLI;

    let upid_str = do_sync_job(job, sync_job, &auth_id, None, to_stdout)?;

    Ok(upid_str)
}

#[sortable]
const SYNC_INFO_SUBDIRS: SubdirMap = &[("run", &Router::new().post(&API_METHOD_RUN_SYNC_JOB))];

const SYNC_INFO_ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(SYNC_INFO_SUBDIRS))
    .subdirs(SYNC_INFO_SUBDIRS);

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_LIST_SYNC_JOBS)
    .match_all("id", &SYNC_INFO_ROUTER);
