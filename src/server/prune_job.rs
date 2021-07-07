use anyhow::Error;

use proxmox::try_block;

use pbs_datastore::{task_log, task_warn};

use crate::{
    api2::types::*,
    backup::{compute_prune_info, BackupInfo, DataStore, PruneOptions},
    server::jobstate::Job,
    server::WorkerTask,
};

pub fn do_prune_job(
    mut job: Job,
    prune_options: PruneOptions,
    store: String,
    auth_id: &Authid,
    schedule: Option<String>,
) -> Result<String, Error> {
    let datastore = DataStore::lookup_datastore(&store)?;

    let worker_type = job.jobtype().to_string();
    let upid_str = WorkerTask::new_thread(
        &worker_type,
        Some(job.jobname().to_string()),
        auth_id.clone(),
        false,
        move |worker| {
            job.start(&worker.upid().to_string())?;

            let result = try_block!({
                task_log!(worker, "Starting datastore prune on store \"{}\"", store);

                if let Some(event_str) = schedule {
                    task_log!(worker, "task triggered by schedule '{}'", event_str);
                }

                task_log!(
                    worker,
                    "retention options: {}",
                    prune_options.cli_options_string()
                );

                let base_path = datastore.base_path();

                let groups = BackupInfo::list_backup_groups(&base_path)?;
                for group in groups {
                    let list = group.list_backups(&base_path)?;
                    let mut prune_info = compute_prune_info(list, &prune_options)?;
                    prune_info.reverse(); // delete older snapshots first

                    task_log!(
                        worker,
                        "Starting prune on store \"{}\" group \"{}/{}\"",
                        store,
                        group.backup_type(),
                        group.backup_id()
                    );

                    for (info, keep) in prune_info {
                        task_log!(
                            worker,
                            "{} {}/{}/{}",
                            if keep { "keep" } else { "remove" },
                            group.backup_type(),
                            group.backup_id(),
                            info.backup_dir.backup_time_string()
                        );
                        if !keep {
                            if let Err(err) = datastore.remove_backup_dir(&info.backup_dir, false) {
                                task_warn!(
                                    worker,
                                    "failed to remove dir {:?}: {}",
                                    info.backup_dir.relative_path(),
                                    err,
                                );
                            }
                        }
                    }
                }
                Ok(())
            });

            let status = worker.create_state(&result);

            if let Err(err) = job.finish(status) {
                eprintln!(
                    "could not finish job state for {}: {}",
                    job.jobtype().to_string(),
                    err
                );
            }

            result
        },
    )?;
    Ok(upid_str)
}
