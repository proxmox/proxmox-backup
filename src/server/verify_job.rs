use anyhow::{format_err, Error};

use crate::{
    server::WorkerTask,
    api2::types::*,
    server::jobstate::Job,
    config::verify::VerificationJobConfig,
    backup::{
        DataStore,
        BackupManifest,
        verify_all_backups,
    },
    task_log,
};

/// Runs a verification job.
pub fn do_verification_job(
    mut job: Job,
    verification_job: VerificationJobConfig,
    auth_id: &Authid,
    schedule: Option<String>,
) -> Result<String, Error> {

    let datastore = DataStore::lookup_datastore(&verification_job.store)?;

    let outdated_after = verification_job.outdated_after.clone();
    let ignore_verified_snapshots = verification_job.ignore_verified.unwrap_or(true);

    let filter = move |manifest: &BackupManifest| {
        if !ignore_verified_snapshots {
            return true;
        }

        let raw_verify_state = manifest.unprotected["verify_state"].clone();
        match serde_json::from_value::<SnapshotVerifyState>(raw_verify_state) {
            Err(_) => return true, // no last verification, always include
            Ok(last_verify) => {
                match outdated_after {
                    None => false, // never re-verify if ignored and no max age
                    Some(max_age) => {
                        let now = proxmox::tools::time::epoch_i64();
                        let days_since_last_verify = (now - last_verify.upid.starttime) / 86400;

                        days_since_last_verify > max_age
                    }
                }
            }
        }
    };

    let (email, notify) = crate::server::lookup_datastore_notify_settings(&verification_job.store);

    let job_id = format!("{}:{}",
                         &verification_job.store,
                         job.jobname());
    let worker_type = job.jobtype().to_string();
    let upid_str = WorkerTask::new_thread(
        &worker_type,
        Some(job_id.clone()),
        auth_id.clone(),
        false,
        move |worker| {
            job.start(&worker.upid().to_string())?;

            task_log!(worker,"Starting datastore verify job '{}'", job_id);
            if let Some(event_str) = schedule {
                task_log!(worker,"task triggered by schedule '{}'", event_str);
            }

            let result = verify_all_backups(datastore, worker.clone(), worker.upid(), None, Some(&filter));
            let job_result = match result {
                Ok(ref errors) if errors.is_empty() => Ok(()),
                Ok(_) => Err(format_err!("verification failed - please check the log for details")),
                Err(_) => Err(format_err!("verification failed - job aborted")),
            };

            let status = worker.create_state(&job_result);

            match job.finish(status) {
                Err(err) => eprintln!(
                    "could not finish job state for {}: {}",
                    job.jobtype().to_string(),
                    err
                ),
                Ok(_) => (),
            }

            if let Some(email) = email {
                if let Err(err) = crate::server::send_verify_status(&email, notify, verification_job, &result) {
                    eprintln!("send verify notification failed: {}", err);
                }
            }

            job_result
        },
    )?;
    Ok(upid_str)
}
