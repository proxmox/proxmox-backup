//! Datastore Verify Job Management

use anyhow::{format_err, Error};
use serde_json::Value;

use proxmox_router::{
    list_subdirs_api_method, ApiMethod, Permission, Router, RpcEnvironment, RpcEnvironmentType,
    SubdirMap,
};
use proxmox_schema::api;
use proxmox_sortable_macro::sortable;

use pbs_api_types::{
    Authid, VerificationJobConfig, VerificationJobStatus, DATASTORE_SCHEMA, JOB_ID_SCHEMA,
    PRIV_DATASTORE_AUDIT, PRIV_DATASTORE_VERIFY,
};
use pbs_config::verify;
use pbs_config::CachedUserInfo;

use crate::server::{
    do_verification_job,
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
        items: { type: VerificationJobStatus },
    },
    access: {
        permission: &Permission::Anybody,
        description: "Requires Datastore.Audit or Datastore.Verify on datastore.",
    },
)]
/// List all verification jobs
pub fn list_verification_jobs(
    store: Option<String>,
    _param: Value,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<VerificationJobStatus>, Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let user_info = CachedUserInfo::new()?;

    let required_privs = PRIV_DATASTORE_AUDIT | PRIV_DATASTORE_VERIFY;

    let (config, digest) = verify::config()?;

    let job_config_iter = config
        .convert_to_typed_array("verification")?
        .into_iter()
        .filter(|job: &VerificationJobConfig| {
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
        let last_state = JobState::load("verificationjob", &job.id)
            .map_err(|err| format_err!("could not open statefile for {}: {}", &job.id, err))?;

        let status = compute_schedule_status(&last_state, job.schedule.as_deref())?;

        list.push(VerificationJobStatus {
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
        description: "Requires Datastore.Verify on job's datastore.",
    },
)]
/// Runs a verification job manually.
pub fn run_verification_job(
    id: String,
    _info: &ApiMethod,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<String, Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let user_info = CachedUserInfo::new()?;

    let (config, _digest) = verify::config()?;
    let verification_job: VerificationJobConfig = config.lookup("verification", &id)?;

    user_info.check_privs(
        &auth_id,
        &verification_job.acl_path(),
        PRIV_DATASTORE_VERIFY,
        true,
    )?;

    let job = Job::new("verificationjob", &id)?;
    let to_stdout = rpcenv.env_type() == RpcEnvironmentType::CLI;

    let upid_str = do_verification_job(job, verification_job, &auth_id, None, to_stdout)?;

    Ok(upid_str)
}

#[sortable]
const VERIFICATION_INFO_SUBDIRS: SubdirMap =
    &[("run", &Router::new().post(&API_METHOD_RUN_VERIFICATION_JOB))];

const VERIFICATION_INFO_ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(VERIFICATION_INFO_SUBDIRS))
    .subdirs(VERIFICATION_INFO_SUBDIRS);

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_LIST_VERIFICATION_JOBS)
    .match_all("id", &VERIFICATION_INFO_ROUTER);
