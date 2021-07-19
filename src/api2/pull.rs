//! Sync datastore from remote server
use std::sync::{Arc};

use anyhow::{format_err, Error};
use futures::{select, future::FutureExt};

use proxmox::api::api;
use proxmox::api::{ApiMethod, Router, RpcEnvironment, Permission};

use pbs_client::{HttpClient, BackupRepository};

use crate::server::{WorkerTask, jobstate::Job, pull::pull_store};
use crate::backup::DataStore;
use crate::api2::types::{
    DATASTORE_SCHEMA, REMOTE_ID_SCHEMA, REMOVE_VANISHED_BACKUPS_SCHEMA, Authid,
};
use crate::config::{
    remote,
    sync::SyncJobConfig,
    acl::{PRIV_DATASTORE_BACKUP, PRIV_DATASTORE_PRUNE, PRIV_REMOTE_READ},
    cached_user_info::CachedUserInfo,
};


pub fn check_pull_privs(
    auth_id: &Authid,
    store: &str,
    remote: &str,
    remote_store: &str,
    delete: bool,
) -> Result<(), Error> {

    let user_info = CachedUserInfo::new()?;

    user_info.check_privs(auth_id, &["datastore", store], PRIV_DATASTORE_BACKUP, false)?;
    user_info.check_privs(auth_id, &["remote", remote, remote_store], PRIV_REMOTE_READ, false)?;

    if delete {
        user_info.check_privs(auth_id, &["datastore", store], PRIV_DATASTORE_PRUNE, false)?;
    }

    Ok(())
}

pub async fn get_pull_parameters(
    store: &str,
    remote: &str,
    remote_store: &str,
) -> Result<(HttpClient, BackupRepository, Arc<DataStore>), Error> {

    let tgt_store = DataStore::lookup_datastore(store)?;

    let (remote_config, _digest) = remote::config()?;
    let remote: remote::Remote = remote_config.lookup("remote", remote)?;

    let src_repo = BackupRepository::new(Some(remote.auth_id.clone()), Some(remote.host.clone()), remote.port, remote_store.to_string());

    let client = crate::api2::config::remote::remote_client(remote).await?;

    Ok((client, src_repo, tgt_store))
}

pub fn do_sync_job(
    mut job: Job,
    sync_job: SyncJobConfig,
    auth_id: &Authid,
    schedule: Option<String>,
) -> Result<String, Error> {

    let job_id = format!("{}:{}:{}:{}",
                         sync_job.remote,
                         sync_job.remote_store,
                         sync_job.store,
                         job.jobname());
    let worker_type = job.jobtype().to_string();

    let (email, notify) = crate::server::lookup_datastore_notify_settings(&sync_job.store);

    let upid_str = WorkerTask::spawn(
        &worker_type,
        Some(job_id.clone()),
        auth_id.clone(),
        false,
        move |worker| async move {

            job.start(&worker.upid().to_string())?;

            let worker2 = worker.clone();
            let sync_job2 = sync_job.clone();

            let worker_future = async move {

                let delete = sync_job.remove_vanished.unwrap_or(true);
                let sync_owner = sync_job.owner.unwrap_or_else(|| Authid::root_auth_id().clone());
                let (client, src_repo, tgt_store) = get_pull_parameters(&sync_job.store, &sync_job.remote, &sync_job.remote_store).await?;

                worker.log(format!("Starting datastore sync job '{}'", job_id));
                if let Some(event_str) = schedule {
                    worker.log(format!("task triggered by schedule '{}'", event_str));
                }
                worker.log(format!("Sync datastore '{}' from '{}/{}'",
                        sync_job.store, sync_job.remote, sync_job.remote_store));

                pull_store(&worker, &client, &src_repo, tgt_store.clone(), delete, sync_owner).await?;

                worker.log(format!("sync job '{}' end", &job_id));

                Ok(())
            };

            let mut abort_future = worker2.abort_future().map(|_| Err(format_err!("sync aborted")));

            let result = select!{
                worker = worker_future.fuse() => worker,
                abort = abort_future => abort,
            };

            let status = worker2.create_state(&result);

            match job.finish(status) {
                Ok(_) => {},
                Err(err) => {
                    eprintln!("could not finish job state: {}", err);
                }
            }

            if let Some(email) = email {
                if let Err(err) = crate::server::send_sync_status(&email, notify, &sync_job2, &result) {
                    eprintln!("send sync notification failed: {}", err);
                }
            }

            result
        })?;

    Ok(upid_str)
}

#[api(
    input: {
        properties: {
            store: {
                schema: DATASTORE_SCHEMA,
            },
            remote: {
                schema: REMOTE_ID_SCHEMA,
            },
            "remote-store": {
                schema: DATASTORE_SCHEMA,
            },
            "remove-vanished": {
                schema: REMOVE_VANISHED_BACKUPS_SCHEMA,
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
async fn pull (
    store: String,
    remote: String,
    remote_store: String,
    remove_vanished: Option<bool>,
    _info: &ApiMethod,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<String, Error> {

    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let delete = remove_vanished.unwrap_or(true);

    check_pull_privs(&auth_id, &store, &remote, &remote_store, delete)?;

    let (client, src_repo, tgt_store) = get_pull_parameters(&store, &remote, &remote_store).await?;

    // fixme: set to_stdout to false?
    let upid_str = WorkerTask::spawn("sync", Some(store.clone()), auth_id.clone(), true, move |worker| async move {

        worker.log(format!("sync datastore '{}' start", store));

        let pull_future = pull_store(&worker, &client, &src_repo, tgt_store.clone(), delete, auth_id);
        let future = select!{
            success = pull_future.fuse() => success,
            abort = worker.abort_future().map(|_| Err(format_err!("pull aborted"))) => abort,
        };

        let _ = future?;

        worker.log(format!("sync datastore '{}' end", store));

        Ok(())
    })?;

    Ok(upid_str)
}

pub const ROUTER: Router = Router::new()
    .post(&API_METHOD_PULL);
