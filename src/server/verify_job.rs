use anyhow::{format_err, Error};

use crate::{
    server::WorkerTask,
    api2::types::*,
    server::jobstate::Job,
    config::verify::VerificationJobConfig,
    backup::{
        DataStore,
        BackupInfo,
        verify_all_backups,
    },
    task_log,
};

/// Runs a verification job.
pub fn do_verification_job(
    mut job: Job,
    verification_job: VerificationJobConfig,
    userid: &Userid,
    schedule: Option<String>,
) -> Result<String, Error> {

    let datastore = DataStore::lookup_datastore(&verification_job.store)?;

    let datastore2 = datastore.clone();

    let outdated_after = verification_job.outdated_after.clone();
    let ignore_verified = verification_job.ignore_verified.unwrap_or(true);

    let filter = move |backup_info: &BackupInfo| {
        if !ignore_verified {
            return true;
        }
        let manifest = match datastore2.load_manifest(&backup_info.backup_dir) {
            Ok((manifest, _)) => manifest,
            Err(_) => return true,
        };

        let raw_verify_state = manifest.unprotected["verify_state"].clone();
        let last_state = match serde_json::from_value::<SnapshotVerifyState>(raw_verify_state) {
            Ok(last_state) => last_state,
            Err(_) => return true,
        };

        let now = proxmox::tools::time::epoch_i64();
        let days_since_last_verify = (now - last_state.upid.starttime) / 86400;

        outdated_after
            .map(|v| days_since_last_verify > v)
            .unwrap_or(true)
    };

    let email = crate::server::lookup_user_email(userid);

    let job_id = job.jobname().to_string();
    let worker_type = job.jobtype().to_string();
    let upid_str = WorkerTask::new_thread(
        &worker_type,
        Some(job.jobname().to_string()),
        userid.clone(),
        false,
        move |worker| {
            job.start(&worker.upid().to_string())?;

            task_log!(worker,"Starting datastore verify job '{}'", job_id);
            if let Some(event_str) = schedule {
                task_log!(worker,"task triggered by schedule '{}'", event_str);
            }

            let result = verify_all_backups(datastore, worker.clone(), worker.upid(), &filter);
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
                if let Err(err) = crate::server::send_verify_status(&email, verification_job, &result) {
                    eprintln!("send verify notification failed: {}", err);
                }
            }

            job_result
        },
    )?;
    Ok(upid_str)
}
