use std::sync::Arc;
use anyhow::Error;

use crate::{
    server::WorkerTask,
    api2::types::*,
    server::jobstate::Job,
    backup::DataStore,
};

/// Runs a garbage collection job.
pub fn do_garbage_collection_job(
    mut job: Job,
    datastore: Arc<DataStore>,
    auth_id: &Authid,
    schedule: Option<String>,
) -> Result<String, Error> {

    let email = crate::server::lookup_user_email(auth_id.user());

    let store = datastore.name().to_string();

    let worker_type = job.jobtype().to_string();
    let upid_str = WorkerTask::new_thread(
        &worker_type,
        Some(store.clone()),
        auth_id.clone(),
        false,
        move |worker| {
            job.start(&worker.upid().to_string())?;

            worker.log(format!("starting garbage collection on store {}", store));
            if let Some(event_str) = schedule {
                worker.log(format!("task triggered by schedule '{}'", event_str));
            }

            let result = datastore.garbage_collection(&*worker, worker.upid());

            let status = worker.create_state(&result);

            match job.finish(status) {
                Err(err) => eprintln!(
                    "could not finish job state for {}: {}",
                    job.jobtype().to_string(),
                    err
                ),
                Ok(_) => (),
            }

            if let Some(email) = email {
                let gc_status = datastore.last_gc_status();
                if let Err(err) = crate::server::send_gc_status(&email, &store, &gc_status, &result) {
                    eprintln!("send gc notification failed: {}", err);
                }
            }

            result
        }
    )?;

    Ok(upid_str)
}
