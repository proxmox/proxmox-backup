use std::collections::HashMap;

use anyhow::{Error};
use serde_json::Value;

use proxmox::api::{api, ApiMethod, Router, RpcEnvironment};
use proxmox::api::router::SubdirMap;
use proxmox::{list_subdirs_api_method, sortable};

use crate::api2::types::*;
use crate::api2::pull::{get_pull_parameters};
use crate::config::sync::{self, SyncJobStatus, SyncJobConfig};
use crate::server::{self, TaskListInfo, WorkerTask};
use crate::tools::systemd::time::{
    parse_calendar_event, compute_next_event};

#[api(
    input: {
        properties: {},
    },
    returns: {
        description: "List configured jobs and their status.",
        type: Array,
        items: { type: sync::SyncJobStatus },
    },
)]
/// List all sync jobs
pub fn list_sync_jobs(
    _param: Value,
    mut rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<SyncJobStatus>, Error> {

    let (config, digest) = sync::config()?;

    let mut list: Vec<SyncJobStatus> = config.convert_to_typed_array("sync")?;

    let mut last_tasks: HashMap<String, &TaskListInfo> = HashMap::new();
    let tasks = server::read_task_list()?;

    for info in tasks.iter() {
        let worker_id = match &info.upid.worker_id {
            Some(id) => id,
            _ => { continue; },
        };
        if let Some(last) = last_tasks.get(worker_id) {
            if last.upid.starttime < info.upid.starttime {
                last_tasks.insert(worker_id.to_string(), &info);
            }
        } else {
            last_tasks.insert(worker_id.to_string(), &info);
        }
    }

    for job in &mut list {
        let mut last = 0;
        if let Some(task) = last_tasks.get(&job.id) {
            job.last_run_upid = Some(task.upid_str.clone());
            if let Some((endtime, status)) = &task.state {
                job.last_run_state = Some(String::from(status));
                job.last_run_endtime = Some(*endtime);
                last = *endtime;
            }
        }

        job.next_run = (|| -> Option<i64> {
            let schedule = job.schedule.as_ref()?;
            let event = parse_calendar_event(&schedule).ok()?;
            compute_next_event(&event, last, false).ok()
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
    }
)]
/// Runs the sync jobs manually.
async fn run_sync_job(
    id: String,
    _info: &ApiMethod,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<String, Error> {

    let (config, _digest) = sync::config()?;
    let sync_job: SyncJobConfig = config.lookup("sync", &id)?;

    let userid: Userid = rpcenv.get_user().unwrap().parse()?;

    let delete = sync_job.remove_vanished.unwrap_or(true);
    let (client, src_repo, tgt_store) = get_pull_parameters(&sync_job.store, &sync_job.remote, &sync_job.remote_store).await?;

    let upid_str = WorkerTask::spawn("syncjob", Some(id.clone()), userid, false, move |worker| async move {

        worker.log(format!("sync job '{}' start", &id));

        crate::client::pull::pull_store(
            &worker,
            &client,
            &src_repo,
            tgt_store.clone(),
            delete,
            Userid::backup_userid().clone(),
        ).await?;

        worker.log(format!("sync job '{}' end", &id));

        Ok(())
    })?;

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
