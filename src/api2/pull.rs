//! Sync datastore from remote server
use anyhow::{bail, format_err, Error};
use futures::{future::FutureExt, select};

use proxmox_router::{Permission, Router, RpcEnvironment};
use proxmox_schema::api;
use proxmox_sys::task_log;

use pbs_api_types::{
    Authid, BackupNamespace, GroupFilter, RateLimitConfig, SyncJobConfig, DATASTORE_SCHEMA,
    GROUP_FILTER_LIST_SCHEMA, NS_MAX_DEPTH_REDUCED_SCHEMA, PRIV_DATASTORE_BACKUP,
    PRIV_DATASTORE_PRUNE, PRIV_REMOTE_READ, REMOTE_ID_SCHEMA, REMOVE_VANISHED_BACKUPS_SCHEMA,
    TRANSFER_LAST_SCHEMA,
};
use pbs_config::CachedUserInfo;
use proxmox_rest_server::WorkerTask;

use crate::server::jobstate::Job;
use crate::server::pull::{pull_store, PullParameters};

pub fn check_pull_privs(
    auth_id: &Authid,
    store: &str,
    ns: Option<&str>,
    remote: Option<&str>,
    remote_store: &str,
    delete: bool,
) -> Result<(), Error> {
    let user_info = CachedUserInfo::new()?;

    let local_store_ns_acl_path = match ns {
        Some(ns) => vec!["datastore", store, ns],
        None => vec!["datastore", store],
    };

    user_info.check_privs(
        auth_id,
        &local_store_ns_acl_path,
        PRIV_DATASTORE_BACKUP,
        false,
    )?;

    if let Some(remote) = remote {
        user_info.check_privs(
            auth_id,
            &["remote", remote, remote_store],
            PRIV_REMOTE_READ,
            false,
        )?;
    } else {
        user_info.check_privs(
            auth_id,
            &["datastore", remote_store],
            PRIV_DATASTORE_BACKUP,
            false,
        )?;
    }

    if delete {
        user_info.check_privs(
            auth_id,
            &local_store_ns_acl_path,
            PRIV_DATASTORE_PRUNE,
            false,
        )?;
    }

    Ok(())
}

impl TryFrom<&SyncJobConfig> for PullParameters {
    type Error = Error;

    fn try_from(sync_job: &SyncJobConfig) -> Result<Self, Self::Error> {
        PullParameters::new(
            &sync_job.store,
            sync_job.ns.clone().unwrap_or_default(),
            sync_job.remote.as_deref(),
            &sync_job.remote_store,
            sync_job.remote_ns.clone().unwrap_or_default(),
            sync_job
                .owner
                .as_ref()
                .unwrap_or_else(|| Authid::root_auth_id())
                .clone(),
            sync_job.remove_vanished,
            sync_job.max_depth,
            sync_job.group_filter.clone(),
            sync_job.limit.clone(),
            sync_job.transfer_last,
        )
    }
}

pub fn do_sync_job(
    mut job: Job,
    sync_job: SyncJobConfig,
    auth_id: &Authid,
    schedule: Option<String>,
    to_stdout: bool,
) -> Result<String, Error> {
    let job_id = format!(
        "{}:{}:{}:{}:{}",
        sync_job.remote.as_deref().unwrap_or("-"),
        sync_job.remote_store,
        sync_job.store,
        sync_job.ns.clone().unwrap_or_default(),
        job.jobname()
    );
    let worker_type = job.jobtype().to_string();

    if sync_job.remote.is_none() && sync_job.store == sync_job.remote_store {
        bail!("can't sync to same datastore");
    }

    let (email, notify) = crate::server::lookup_datastore_notify_settings(&sync_job.store);

    let upid_str = WorkerTask::spawn(
        &worker_type,
        Some(job_id.clone()),
        auth_id.to_string(),
        to_stdout,
        move |worker| async move {
            job.start(&worker.upid().to_string())?;

            let worker2 = worker.clone();
            let sync_job2 = sync_job.clone();

            let worker_future = async move {
                let pull_params = PullParameters::try_from(&sync_job)?;

                task_log!(worker, "Starting datastore sync job '{}'", job_id);
                if let Some(event_str) = schedule {
                    task_log!(worker, "task triggered by schedule '{}'", event_str);
                }
                task_log!(
                    worker,
                    "sync datastore '{}' from '{}{}'",
                    sync_job.store,
                    sync_job
                        .remote
                        .as_deref()
                        .map_or(String::new(), |remote| format!("{remote}/")),
                    sync_job.remote_store,
                );

                pull_store(&worker, pull_params).await?;

                task_log!(worker, "sync job '{}' end", &job_id);

                Ok(())
            };

            let mut abort_future = worker2
                .abort_future()
                .map(|_| Err(format_err!("sync aborted")));

            let result = select! {
                worker = worker_future.fuse() => worker,
                abort = abort_future => abort,
            };

            let status = worker2.create_state(&result);

            match job.finish(status) {
                Ok(_) => {}
                Err(err) => {
                    eprintln!("could not finish job state: {}", err);
                }
            }

            if let Some(email) = email {
                if let Err(err) =
                    crate::server::send_sync_status(&email, notify, &sync_job2, &result)
                {
                    eprintln!("send sync notification failed: {}", err);
                }
            }

            result
        },
    )?;

    Ok(upid_str)
}

#[api(
    input: {
        properties: {
            store: {
                schema: DATASTORE_SCHEMA,
            },
            ns: {
                type: BackupNamespace,
                optional: true,
            },
            remote: {
                schema: REMOTE_ID_SCHEMA,
                optional: true,
            },
            "remote-store": {
                schema: DATASTORE_SCHEMA,
            },
            "remote-ns": {
                type: BackupNamespace,
                optional: true,
            },
            "remove-vanished": {
                schema: REMOVE_VANISHED_BACKUPS_SCHEMA,
                optional: true,
            },
            "max-depth": {
                schema: NS_MAX_DEPTH_REDUCED_SCHEMA,
                optional: true,
            },
            "group-filter": {
                schema: GROUP_FILTER_LIST_SCHEMA,
                optional: true,
            },
            limit: {
                type: RateLimitConfig,
                flatten: true,
            },
            "transfer-last": {
                schema: TRANSFER_LAST_SCHEMA,
                optional: true,
            },
        },
    },
    access: {
        // Note: used parameters are no uri parameters, so we need to test inside function body
        description: r###"The user needs Datastore.Backup privilege on '/datastore/{store}',
and needs to own the backup group. Remote.Read is required on '/remote/{remote}/{remote-store}'.
The delete flag additionally requires the Datastore.Prune privilege on '/datastore/{store}'.
"###,
        permission: &Permission::Anybody,
    },
)]
/// Sync store from other repository
#[allow(clippy::too_many_arguments)]
async fn pull(
    store: String,
    ns: Option<BackupNamespace>,
    remote: Option<String>,
    remote_store: String,
    remote_ns: Option<BackupNamespace>,
    remove_vanished: Option<bool>,
    max_depth: Option<usize>,
    group_filter: Option<Vec<GroupFilter>>,
    limit: RateLimitConfig,
    transfer_last: Option<usize>,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<String, Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let delete = remove_vanished.unwrap_or(false);

    if remote.is_none() && store == remote_store {
        bail!("can't sync to same datastore");
    }

    let ns = ns.unwrap_or_default();
    let ns_str = if ns.is_root() {
        None
    } else {
        Some(ns.to_string())
    };

    check_pull_privs(
        &auth_id,
        &store,
        ns_str.as_deref(),
        remote.as_deref(),
        &remote_store,
        delete,
    )?;

    let pull_params = PullParameters::new(
        &store,
        ns,
        remote.as_deref(),
        &remote_store,
        remote_ns.unwrap_or_default(),
        auth_id.clone(),
        remove_vanished,
        max_depth,
        group_filter,
        limit,
        transfer_last,
    )?;

    // fixme: set to_stdout to false?
    // FIXME: add namespace to worker id?
    let upid_str = WorkerTask::spawn(
        "sync",
        Some(store.clone()),
        auth_id.to_string(),
        true,
        move |worker| async move {
            task_log!(
                worker,
                "pull datastore '{}' from '{}/{}'",
                store,
                remote.as_deref().unwrap_or("-"),
                remote_store,
            );

            let pull_future = pull_store(&worker, pull_params);
            (select! {
                success = pull_future.fuse() => success,
                abort = worker.abort_future().map(|_| Err(format_err!("pull aborted"))) => abort,
            })?;

            task_log!(worker, "pull datastore '{}' end", store);

            Ok(())
        },
    )?;

    Ok(upid_str)
}

pub const ROUTER: Router = Router::new().post(&API_METHOD_PULL);
