//! Datastore Prune Job Management

use anyhow::{format_err, Error};
use serde_json::Value;

use proxmox_router::{
    list_subdirs_api_method, ApiMethod, Permission, Router, RpcEnvironment, SubdirMap,
};
use proxmox_schema::api;
use proxmox_sortable_macro::sortable;

use pbs_api_types::{
    Authid, PruneJobConfig, PruneJobStatus, DATASTORE_SCHEMA, JOB_ID_SCHEMA, PRIV_DATASTORE_AUDIT,
    PRIV_DATASTORE_MODIFY,
};
use pbs_config::prune;
use pbs_config::CachedUserInfo;

use crate::server::{
    do_prune_job,
    jobstate::{compute_schedule_status, Job, JobState},
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
        description: "List configured jobs and their status (filtered by access)",
        type: Array,
        items: { type: PruneJobStatus },
    },
    access: {
        permission: &Permission::Anybody,
        description: "Requires Datastore.Audit or Datastore.Modify on datastore.",
    },
)]
/// List all prune jobs
pub fn list_prune_jobs(
    store: Option<String>,
    _param: Value,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<PruneJobStatus>, Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let user_info = CachedUserInfo::new()?;

    let required_privs = PRIV_DATASTORE_AUDIT | PRIV_DATASTORE_MODIFY;

    let (config, digest) = prune::config()?;

    let job_config_iter =
        config
            .convert_to_typed_array("prune")?
            .into_iter()
            .filter(|job: &PruneJobConfig| {
                let privs = user_info.lookup_privs(&auth_id, &job.acl_path());
                if privs & required_privs == 0 {
                    return false;
                }

                if let Some(store) = &store {
                    &job.store == store
                } else {
                    true
                }
            });

    let mut list = Vec::new();

    for job in job_config_iter {
        let last_state = JobState::load("prunejob", &job.id)
            .map_err(|err| format_err!("could not open statefile for {}: {}", &job.id, err))?;

        let mut status = compute_schedule_status(&last_state, Some(&job.schedule))?;
        if job.disable {
            status.next_run = None;
        }

        list.push(PruneJobStatus {
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
        permission: &Permission::Anybody,
        description: "Requires Datastore.Modify on job's datastore.",
    },
)]
/// Runs a prune job manually.
pub fn run_prune_job(
    id: String,
    _info: &ApiMethod,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<String, Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let user_info = CachedUserInfo::new()?;

    let (config, _digest) = prune::config()?;
    let prune_job: PruneJobConfig = config.lookup("prune", &id)?;

    user_info.check_privs(&auth_id, &prune_job.acl_path(), PRIV_DATASTORE_MODIFY, true)?;

    let job = Job::new("prunejob", &id)?;

    let upid_str = do_prune_job(job, prune_job.options, prune_job.store, &auth_id, None)?;

    Ok(upid_str)
}

#[sortable]
const PRUNE_INFO_SUBDIRS: SubdirMap = &[("run", &Router::new().post(&API_METHOD_RUN_PRUNE_JOB))];

const PRUNE_INFO_ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(PRUNE_INFO_SUBDIRS))
    .subdirs(PRUNE_INFO_SUBDIRS);

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_LIST_PRUNE_JOBS)
    .match_all("id", &PRUNE_INFO_ROUTER);
