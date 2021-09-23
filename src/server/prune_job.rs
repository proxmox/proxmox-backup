use std::sync::Arc;

use anyhow::Error;

use pbs_datastore::backup_info::BackupInfo;
use pbs_datastore::prune::compute_prune_info;
use pbs_api_types::{Authid, PRIV_DATASTORE_MODIFY, PruneOptions};
use pbs_config::CachedUserInfo;
use pbs_tools::{task_log, task_warn};
use proxmox_rest_server::WorkerTask;

use crate::{
    backup::DataStore,
    server::jobstate::Job,
 };

pub fn prune_datastore(
    worker: Arc<WorkerTask>,
    auth_id: Authid,
    prune_options: PruneOptions,
    store: &str,
    datastore: Arc<DataStore>,
    dry_run: bool,
) -> Result<(), Error> {
    task_log!(worker, "Starting datastore prune on store \"{}\"", store);

    if dry_run {
        task_log!(worker, "(dry test run)");
    }

    let keep_all = !pbs_datastore::prune::keeps_something(&prune_options);

    if keep_all {
        task_log!(worker, "No prune selection - keeping all files.");
    } else {
        task_log!(
            worker,
            "retention options: {}",
            pbs_datastore::prune::cli_options_string(&prune_options)
        );
    }

    let user_info = CachedUserInfo::new()?;
    let privs = user_info.lookup_privs(&auth_id, &["datastore", store]);
    let has_privs = privs & PRIV_DATASTORE_MODIFY != 0;

    let base_path = datastore.base_path();

    let groups = BackupInfo::list_backup_groups(&base_path)?;
    for group in groups {
        let list = group.list_backups(&base_path)?;

        if !has_privs && !datastore.owns_backup(&group, &auth_id)? {
            continue;
        }

        let mut prune_info = compute_prune_info(list, &prune_options)?;
        prune_info.reverse(); // delete older snapshots first

        task_log!(
            worker,
            "Starting prune on store \"{}\" group \"{}/{}\"",
            store,
            group.backup_type(),
            group.backup_id()
        );

        for (info, mut keep) in prune_info {
            if keep_all { keep = true; }
            task_log!(
                worker,
                "{} {}/{}/{}",
                if keep { "keep" } else { "remove" },
                group.backup_type(),
                group.backup_id(),
                info.backup_dir.backup_time_string()
            );
            if !keep && !dry_run {
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
}

pub fn do_prune_job(
    mut job: Job,
    prune_options: PruneOptions,
    store: String,
    auth_id: &Authid,
    schedule: Option<String>,
) -> Result<String, Error> {
    let datastore = DataStore::lookup_datastore(&store)?;

    let worker_type = job.jobtype().to_string();
    let auth_id = auth_id.clone();
    let upid_str = WorkerTask::new_thread(
        &worker_type,
        Some(job.jobname().to_string()),
        auth_id.to_string(),
        false,
        move |worker| {
            job.start(&worker.upid().to_string())?;

            if let Some(event_str) = schedule {
                task_log!(worker, "task triggered by schedule '{}'", event_str);
            }

            let result = prune_datastore(worker.clone(), auth_id, prune_options, &store, datastore, false);

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
