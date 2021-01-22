//! Datastore Syncronization Job Management

use anyhow::{bail, format_err, Error};
use serde_json::Value;

use proxmox::api::{api, ApiMethod, Permission, Router, RpcEnvironment};
use proxmox::api::router::SubdirMap;
use proxmox::{list_subdirs_api_method, sortable};

use crate::api2::types::*;
use crate::api2::pull::do_sync_job;
use crate::api2::config::sync::{check_sync_job_modify_access, check_sync_job_read_access};

use crate::config::cached_user_info::CachedUserInfo;
use crate::config::sync::{self, SyncJobStatus, SyncJobConfig};
use crate::server::UPID;
use crate::server::jobstate::{Job, JobState};
use crate::tools::systemd::time::{
    parse_calendar_event, compute_next_event};

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
        items: { type: sync::SyncJobStatus },
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
    mut rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<SyncJobStatus>, Error> {

    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let user_info = CachedUserInfo::new()?;

    let (config, digest) = sync::config()?;

    let mut list: Vec<SyncJobStatus> = config
        .convert_to_typed_array("sync")?
        .into_iter()
        .filter(|job: &SyncJobStatus| {
            if let Some(store) = &store {
                &job.store == store
            } else {
                true
            }
        })
        .filter(|job: &SyncJobStatus| {
            let as_config: SyncJobConfig = job.into();
            check_sync_job_read_access(&user_info, &auth_id, &as_config)
        }).collect();

    for job in &mut list {
        let last_state = JobState::load("syncjob", &job.id)
            .map_err(|err| format_err!("could not open statefile for {}: {}", &job.id, err))?;
        let (upid, endtime, state, starttime) = match last_state {
            JobState::Created { time } => (None, None, None, time),
            JobState::Started { upid } => {
                let parsed_upid: UPID = upid.parse()?;
                (Some(upid), None, None, parsed_upid.starttime)
            },
            JobState::Finished { upid, state } => {
                let parsed_upid: UPID = upid.parse()?;
                (Some(upid), Some(state.endtime()), Some(state.to_string()), parsed_upid.starttime)
            },
        };

        job.last_run_upid = upid;
        job.last_run_state = state;
        job.last_run_endtime = endtime;

        let last = job.last_run_endtime.unwrap_or(starttime);

        job.next_run = (|| -> Option<i64> {
            let schedule = job.schedule.as_ref()?;
            let event = parse_calendar_event(&schedule).ok()?;
            // ignore errors
            compute_next_event(&event, last, false).unwrap_or(None)
        })();
    }

    rpcenv["digest"] = proxmox::tools::digest_to_hex(&digest).into();

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

    let upid_str = do_sync_job(job, sync_job, &auth_id, None)?;

    Ok(upid_str)
}

#[sortable]
const SYNC_INFO_SUBDIRS: SubdirMap = &[
    (
        "run",
        &Router::new()
            .post(&API_METHOD_RUN_SYNC_JOB)
    ),
];

const SYNC_INFO_ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(SYNC_INFO_SUBDIRS))
    .subdirs(SYNC_INFO_SUBDIRS);


pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_LIST_SYNC_JOBS)
    .match_all("id", &SYNC_INFO_ROUTER);
