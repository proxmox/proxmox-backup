use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use anyhow::{bail, Error};

use crate::{
    server::WorkerTask,
    api2::types::*,
    server::jobstate::Job,
    config::verify::VerificationJobConfig,
    backup::{
        DataStore,
        BackupInfo,
        verify_backup_dir,
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

    let mut backups_to_verify = BackupInfo::list_backups(&datastore.base_path())?;
    if verification_job.ignore_verified.unwrap_or(true) {
        backups_to_verify.retain(|backup_info| {
            let manifest = match datastore.load_manifest(&backup_info.backup_dir) {
                Ok((manifest, _)) => manifest,
                Err(_) => return false,
            };

            let raw_verify_state = manifest.unprotected["verify_state"].clone();
            let last_state = match serde_json::from_value::<SnapshotVerifyState>(raw_verify_state) {
                Ok(last_state) => last_state,
                Err(_) => return true,
            };

            let now = proxmox::tools::time::epoch_i64();
            let days_since_last_verify = (now - last_state.upid.starttime) / 86400;
            verification_job.outdated_after.is_some()
                && days_since_last_verify > verification_job.outdated_after.unwrap()
        })
    }

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
            task_log!(worker,"verifying {} backups", backups_to_verify.len());
            if let Some(event_str) = schedule {
                task_log!(worker,"task triggered by schedule '{}'", event_str);
            }

            let verified_chunks = Arc::new(Mutex::new(HashSet::with_capacity(1024 * 16)));
            let corrupt_chunks = Arc::new(Mutex::new(HashSet::with_capacity(64)));
            let result = proxmox::try_block!({
                let mut failed_dirs: Vec<String> = Vec::new();

                for backup_info in backups_to_verify {
                    let verification_result = verify_backup_dir(
                        datastore.clone(),
                        &backup_info.backup_dir,
                        verified_chunks.clone(),
                        corrupt_chunks.clone(),
                        worker.clone(),
                        worker.upid().clone()
                    );

                    if let Ok(false) = verification_result {
                        failed_dirs.push(backup_info.backup_dir.to_string());
                    } // otherwise successful or aborted
                }

                if !failed_dirs.is_empty() {
                    task_log!(worker,"Failed to verify following snapshots:",);
                    for dir in failed_dirs {
                        task_log!(worker, "\t{}", dir)
                    }
                    bail!("verification failed - please check the log for details");
                }
                Ok(())
            });

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
                if let Err(err) = crate::server::send_verify_status(&email, verification_job, &result) {
                    eprintln!("send verify notification failed: {}", err);
                }
            }

            result
        },
    )?;
    Ok(upid_str)
}
