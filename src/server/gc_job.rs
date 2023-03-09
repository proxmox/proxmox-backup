use anyhow::Error;
use std::sync::Arc;

use proxmox_sys::task_log;

use pbs_api_types::Authid;
use pbs_datastore::DataStore;
use proxmox_rest_server::WorkerTask;

use crate::server::{jobstate::Job, send_gc_status};

/// Runs a garbage collection job.
pub fn do_garbage_collection_job(
    mut job: Job,
    datastore: Arc<DataStore>,
    auth_id: &Authid,
    schedule: Option<String>,
    to_stdout: bool,
) -> Result<String, Error> {
    let store = datastore.name().to_string();

    let (email, notify) = crate::server::lookup_datastore_notify_settings(&store);

    let worker_type = job.jobtype().to_string();
    let upid_str = WorkerTask::new_thread(
        &worker_type,
        Some(store.clone()),
        auth_id.to_string(),
        to_stdout,
        move |worker| {
            job.start(&worker.upid().to_string())?;

            task_log!(worker, "starting garbage collection on store {store}");
            if let Some(event_str) = schedule {
                task_log!(worker, "task triggered by schedule '{event_str}'");
            }

            let result = datastore.garbage_collection(&*worker, worker.upid());

            let status = worker.create_state(&result);

            if let Err(err) = job.finish(status) {
                eprintln!("could not finish job state for {}: {err}", job.jobtype());
            }

            if let Some(email) = email {
                let gc_status = datastore.last_gc_status();
                if let Err(err) = send_gc_status(&email, notify, &store, &gc_status, &result) {
                    eprintln!("send gc notification failed: {err}");
                }
            }

            result
        },
    )?;

    Ok(upid_str)
}
