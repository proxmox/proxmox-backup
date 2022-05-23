use std::sync::Arc;

use anyhow::Error;

use proxmox_sys::{task_log, task_warn};

use pbs_api_types::{
    Authid, BackupNamespace, Operation, PruneOptions, PRIV_DATASTORE_MODIFY, PRIV_DATASTORE_PRUNE,
};
use pbs_datastore::prune::compute_prune_info;
use pbs_datastore::DataStore;
use proxmox_rest_server::WorkerTask;

use crate::backup::ListAccessibleBackupGroups;
use crate::server::jobstate::Job;

pub fn prune_datastore(
    worker: Arc<WorkerTask>,
    auth_id: Authid,
    prune_options: PruneOptions,
    datastore: Arc<DataStore>,
    ns: BackupNamespace,
    max_depth: usize,
    dry_run: bool,
) -> Result<(), Error> {
    let store = &datastore.name();
    let depth_str = if max_depth == pbs_api_types::MAX_NAMESPACE_DEPTH {
        " down to full depth".to_string()
    } else if max_depth > 0 {
        format!("to depth {max_depth}")
    } else {
        "non-recursive".to_string()
    };
    if ns.is_root() {
        task_log!(
            worker,
            "Starting datastore prune on store '{store}', {depth_str}"
        );
    } else {
        task_log!(
            worker,
            "Starting datastore prune on store '{store}' namespace '{ns}', {depth_str}"
        );
    }

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

    for group in ListAccessibleBackupGroups::new_with_privs(
        &datastore,
        ns.clone(),
        max_depth,
        Some(PRIV_DATASTORE_MODIFY), // overides the owner check
        Some(PRIV_DATASTORE_PRUNE),  // additionally required if owner
        Some(&auth_id),
    )? {
        let group = group?;
        let ns = group.backup_ns();
        let list = group.list_backups()?;

        let mut prune_info = compute_prune_info(list, &prune_options)?;
        prune_info.reverse(); // delete older snapshots first

        task_log!(
            worker,
            "Pruning group {ns}:\"{}/{}\"",
            group.backup_type(),
            group.backup_id()
        );

        for (info, mark) in prune_info {
            let keep = keep_all || mark.keep();
            task_log!(
                worker,
                "{}{} {}/{}/{}",
                if dry_run { "would " } else { "" },
                mark,
                group.backup_type(),
                group.backup_id(),
                info.backup_dir.backup_time_string()
            );
            if !keep && !dry_run {
                if let Err(err) = datastore.remove_backup_dir(ns, info.backup_dir.as_ref(), false) {
                    let path = info.backup_dir.relative_path();
                    task_warn!(worker, "failed to remove dir {path:?}: {err}");
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
    let datastore = DataStore::lookup_datastore(&store, Some(Operation::Write))?;

    let worker_type = job.jobtype().to_string();
    let auth_id = auth_id.clone();
    let worker_id = format!("{store}");
    let upid_str = WorkerTask::new_thread(
        &worker_type,
        Some(worker_id),
        auth_id.to_string(),
        false,
        move |worker| {
            job.start(&worker.upid().to_string())?;

            task_log!(worker, "prune job '{}'", job.jobname());

            if let Some(event_str) = schedule {
                task_log!(worker, "task triggered by schedule '{}'", event_str);
            }

            let result = prune_datastore(
                worker.clone(),
                auth_id,
                prune_options,
                datastore,
                BackupNamespace::default(),
                pbs_api_types::MAX_NAMESPACE_DEPTH,
                false,
            );

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
