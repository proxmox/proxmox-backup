use std::sync::{Arc, Mutex};

use anyhow::{bail, format_err, Error};
use serde_json::Value;

use proxmox_lang::try_block;
use proxmox_router::{Permission, Router, RpcEnvironment, RpcEnvironmentType};
use proxmox_schema::api;
use proxmox_sys::{task_log, task_warn, WorkerTaskContext};

use pbs_api_types::{
    print_ns_and_snapshot, print_store_and_ns, Authid, MediaPoolConfig, Operation,
    TapeBackupJobConfig, TapeBackupJobSetup, TapeBackupJobStatus, Userid, JOB_ID_SCHEMA,
    PRIV_DATASTORE_READ, PRIV_TAPE_AUDIT, PRIV_TAPE_WRITE, UPID_SCHEMA,
};

use pbs_config::CachedUserInfo;
use pbs_datastore::backup_info::{BackupDir, BackupInfo};
use pbs_datastore::{DataStore, StoreProgress};
use proxmox_rest_server::WorkerTask;

use crate::{
    server::{
        jobstate::{compute_schedule_status, Job, JobState},
        lookup_user_email, TapeBackupJobSummary,
    },
    tape::{
        changer::update_changer_online_status,
        drive::{lock_tape_device, media_changer, set_tape_device_state, TapeLockError},
        Inventory, MediaPool, PoolWriter, TAPE_STATUS_DIR,
    },
};

const TAPE_BACKUP_JOB_ROUTER: Router = Router::new().post(&API_METHOD_RUN_TAPE_BACKUP_JOB);

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_LIST_TAPE_BACKUP_JOBS)
    .post(&API_METHOD_BACKUP)
    .match_all("id", &TAPE_BACKUP_JOB_ROUTER);

fn check_backup_permission(
    auth_id: &Authid,
    store: &str,
    pool: &str,
    drive: &str,
) -> Result<(), Error> {
    let user_info = CachedUserInfo::new()?;

    user_info.check_privs(auth_id, &["datastore", store], PRIV_DATASTORE_READ, false)?;

    user_info.check_privs(auth_id, &["tape", "drive", drive], PRIV_TAPE_WRITE, false)?;

    user_info.check_privs(auth_id, &["tape", "pool", pool], PRIV_TAPE_WRITE, false)?;

    Ok(())
}

#[api(
    returns: {
        description: "List configured thape backup jobs and their status",
        type: Array,
        items: { type: TapeBackupJobStatus },
    },
    access: {
        description: "List configured tape jobs filtered by Tape.Audit privileges",
        permission: &Permission::Anybody,
    },
)]
/// List all tape backup jobs
pub fn list_tape_backup_jobs(
    _param: Value,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<TapeBackupJobStatus>, Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let user_info = CachedUserInfo::new()?;

    let (job_config, digest) = pbs_config::tape_job::config()?;
    let (pool_config, _pool_digest) = pbs_config::media_pool::config()?;
    let (drive_config, _digest) = pbs_config::drive::config()?;

    let job_list_iter = job_config
        .convert_to_typed_array("backup")?
        .into_iter()
        .filter(|_job: &TapeBackupJobConfig| {
            // fixme: check access permission
            true
        });

    let mut list = Vec::new();
    let current_time = proxmox_time::epoch_i64();

    for job in job_list_iter {
        let privs = user_info.lookup_privs(&auth_id, &["tape", "job", &job.id]);
        if (privs & PRIV_TAPE_AUDIT) == 0 {
            continue;
        }

        let last_state = JobState::load("tape-backup-job", &job.id)
            .map_err(|err| format_err!("could not open statefile for {}: {}", &job.id, err))?;

        let status = compute_schedule_status(&last_state, job.schedule.as_deref())?;

        let next_run = status.next_run.unwrap_or(current_time);

        let mut next_media_label = None;

        if let Ok(pool) = pool_config.lookup::<MediaPoolConfig>("pool", &job.setup.pool) {
            let mut changer_name = None;
            if let Ok(Some((_, name))) = media_changer(&drive_config, &job.setup.drive) {
                changer_name = Some(name);
            }
            if let Ok(mut pool) = MediaPool::with_config(TAPE_STATUS_DIR, &pool, changer_name, true)
            {
                if pool.start_write_session(next_run, false).is_ok() {
                    if let Ok(media_id) = pool.guess_next_writable_media(next_run) {
                        next_media_label = Some(media_id.label.label_text);
                    }
                }
            }
        }

        list.push(TapeBackupJobStatus {
            config: job,
            status,
            next_media_label,
        });
    }

    rpcenv["digest"] = hex::encode(digest).into();

    Ok(list)
}

pub fn do_tape_backup_job(
    mut job: Job,
    setup: TapeBackupJobSetup,
    auth_id: &Authid,
    schedule: Option<String>,
    to_stdout: bool,
) -> Result<String, Error> {
    let job_id = format!(
        "{}:{}:{}:{}",
        setup.store,
        setup.pool,
        setup.drive,
        job.jobname()
    );

    let worker_type = job.jobtype().to_string();

    let datastore = DataStore::lookup_datastore(&setup.store, Some(Operation::Read))?;

    let (config, _digest) = pbs_config::media_pool::config()?;
    let pool_config: MediaPoolConfig = config.lookup("pool", &setup.pool)?;

    let (drive_config, _digest) = pbs_config::drive::config()?;

    // for scheduled jobs we acquire the lock later in the worker
    let drive_lock = if schedule.is_some() {
        None
    } else {
        Some(lock_tape_device(&drive_config, &setup.drive)?)
    };

    let notify_user = setup
        .notify_user
        .as_ref()
        .unwrap_or_else(|| Userid::root_userid());
    let email = lookup_user_email(notify_user);

    let upid_str = WorkerTask::new_thread(
        &worker_type,
        Some(job_id.clone()),
        auth_id.to_string(),
        to_stdout,
        move |worker| {
            job.start(&worker.upid().to_string())?;
            let mut drive_lock = drive_lock;

            let mut summary = Default::default();
            let job_result = try_block!({
                if schedule.is_some() {
                    // for scheduled tape backup jobs, we wait indefinitely for the lock
                    task_log!(worker, "waiting for drive lock...");
                    loop {
                        worker.check_abort()?;
                        match lock_tape_device(&drive_config, &setup.drive) {
                            Ok(lock) => {
                                drive_lock = Some(lock);
                                break;
                            }
                            Err(TapeLockError::TimeOut) => continue,
                            Err(TapeLockError::Other(err)) => return Err(err),
                        }
                    }
                }
                set_tape_device_state(&setup.drive, &worker.upid().to_string())?;

                task_log!(worker, "Starting tape backup job '{}'", job_id);
                if let Some(event_str) = schedule {
                    task_log!(worker, "task triggered by schedule '{}'", event_str);
                }

                backup_worker(
                    &worker,
                    datastore,
                    &pool_config,
                    &setup,
                    email.clone(),
                    &mut summary,
                    false,
                )
            });

            let status = worker.create_state(&job_result);

            if let Some(email) = email {
                if let Err(err) = crate::server::send_tape_backup_status(
                    &email,
                    Some(job.jobname()),
                    &setup,
                    &job_result,
                    summary,
                ) {
                    eprintln!("send tape backup notification failed: {}", err);
                }
            }

            if let Err(err) = job.finish(status) {
                eprintln!("could not finish job state for {}: {}", job.jobtype(), err);
            }

            if let Err(err) = set_tape_device_state(&setup.drive, "") {
                eprintln!("could not unset drive state for {}: {}", setup.drive, err);
            }

            job_result
        },
    )?;

    Ok(upid_str)
}

#[api(
    input: {
        properties: {
            id: {
                schema: JOB_ID_SCHEMA,
            },
        },
    },
    access: {
        // Note: parameters are from job config, so we need to test inside function body
        description: "The user needs Tape.Write privilege on /tape/pool/{pool} \
                      and /tape/drive/{drive}, Datastore.Read privilege on /datastore/{store}.",
        permission: &Permission::Anybody,
    },
)]
/// Runs a tape backup job manually.
pub fn run_tape_backup_job(id: String, rpcenv: &mut dyn RpcEnvironment) -> Result<String, Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;

    let (config, _digest) = pbs_config::tape_job::config()?;
    let backup_job: TapeBackupJobConfig = config.lookup("backup", &id)?;

    check_backup_permission(
        &auth_id,
        &backup_job.setup.store,
        &backup_job.setup.pool,
        &backup_job.setup.drive,
    )?;

    let job = Job::new("tape-backup-job", &id)?;

    let to_stdout = rpcenv.env_type() == RpcEnvironmentType::CLI;

    let upid_str = do_tape_backup_job(job, backup_job.setup, &auth_id, None, to_stdout)?;

    Ok(upid_str)
}

#[api(
    input: {
        properties: {
            setup: {
                type: TapeBackupJobSetup,
                flatten: true,
            },
            "force-media-set": {
                description: "Ignore the allocation policy and start a new media-set.",
                optional: true,
                type: bool,
                default: false,
            },
        },
    },
    returns: {
        schema: UPID_SCHEMA,
    },
    access: {
        // Note: parameters are no uri parameter, so we need to test inside function body
        description: "The user needs Tape.Write privilege on /tape/pool/{pool} \
                      and /tape/drive/{drive}, Datastore.Read privilege on /datastore/{store}.",
        permission: &Permission::Anybody,
    },
)]
/// Backup datastore to tape media pool
pub fn backup(
    setup: TapeBackupJobSetup,
    force_media_set: bool,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;

    check_backup_permission(&auth_id, &setup.store, &setup.pool, &setup.drive)?;

    let datastore = DataStore::lookup_datastore(&setup.store, Some(Operation::Read))?;

    let (config, _digest) = pbs_config::media_pool::config()?;
    let pool_config: MediaPoolConfig = config.lookup("pool", &setup.pool)?;

    let (drive_config, _digest) = pbs_config::drive::config()?;

    // early check/lock before starting worker
    let drive_lock = lock_tape_device(&drive_config, &setup.drive)?;

    let to_stdout = rpcenv.env_type() == RpcEnvironmentType::CLI;

    let job_id = format!("{}:{}:{}", setup.store, setup.pool, setup.drive);

    let notify_user = setup
        .notify_user
        .as_ref()
        .unwrap_or_else(|| Userid::root_userid());
    let email = lookup_user_email(notify_user);

    let upid_str = WorkerTask::new_thread(
        "tape-backup",
        Some(job_id),
        auth_id.to_string(),
        to_stdout,
        move |worker| {
            let _drive_lock = drive_lock; // keep lock guard
            set_tape_device_state(&setup.drive, &worker.upid().to_string())?;

            let mut summary = Default::default();
            let job_result = backup_worker(
                &worker,
                datastore,
                &pool_config,
                &setup,
                email.clone(),
                &mut summary,
                force_media_set,
            );

            if let Some(email) = email {
                if let Err(err) = crate::server::send_tape_backup_status(
                    &email,
                    None,
                    &setup,
                    &job_result,
                    summary,
                ) {
                    eprintln!("send tape backup notification failed: {}", err);
                }
            }

            // ignore errors
            let _ = set_tape_device_state(&setup.drive, "");
            job_result
        },
    )?;

    Ok(upid_str.into())
}

enum SnapshotBackupResult {
    Success,
    Error,
    Ignored,
}

fn backup_worker(
    worker: &WorkerTask,
    datastore: Arc<DataStore>,
    pool_config: &MediaPoolConfig,
    setup: &TapeBackupJobSetup,
    email: Option<String>,
    summary: &mut TapeBackupJobSummary,
    force_media_set: bool,
) -> Result<(), Error> {
    let start = std::time::Instant::now();

    task_log!(worker, "update media online status");
    let changer_name = update_media_online_status(&setup.drive)?;

    let root_namespace = setup.ns.clone().unwrap_or_default();
    let ns_magic = !root_namespace.is_root() || setup.max_depth != Some(0);

    let pool = MediaPool::with_config(TAPE_STATUS_DIR, pool_config, changer_name, false)?;

    let mut pool_writer =
        PoolWriter::new(pool, &setup.drive, worker, email, force_media_set, ns_magic)?;

    let mut group_list = Vec::new();
    let namespaces = datastore.recursive_iter_backup_ns_ok(root_namespace, setup.max_depth)?;
    for ns in namespaces {
        group_list.extend(datastore.list_backup_groups(ns)?);
    }

    group_list.sort_unstable_by(|a, b| a.group().cmp(b.group()));

    let group_count_full = group_list.len();

    let group_list = match &setup.group_filter {
        Some(f) => group_list
            .into_iter()
            .filter(|group| group.group().apply_filters(f))
            .collect(),
        None => group_list,
    };

    task_log!(
        worker,
        "found {} groups (out of {} total)",
        group_list.len(),
        group_count_full
    );

    let mut progress = StoreProgress::new(group_list.len() as u64);

    let latest_only = setup.latest_only.unwrap_or(false);

    if latest_only {
        task_log!(
            worker,
            "latest-only: true (only considering latest snapshots)"
        );
    }

    let datastore_name = datastore.name();

    let mut errors = false;

    let mut need_catalog = false; // avoid writing catalog for empty jobs

    for (group_number, group) in group_list.into_iter().enumerate() {
        progress.done_groups = group_number as u64;
        progress.done_snapshots = 0;
        progress.group_snapshots = 0;

        let snapshot_list = group.list_backups()?;

        // filter out unfinished backups
        let mut snapshot_list: Vec<_> = snapshot_list
            .into_iter()
            .filter(|item| item.is_finished())
            .collect();

        if snapshot_list.is_empty() {
            task_log!(
                worker,
                "{}, group {} was empty",
                print_store_and_ns(datastore_name, group.backup_ns()),
                group.group()
            );
            continue;
        }

        BackupInfo::sort_list(&mut snapshot_list, true); // oldest first

        if latest_only {
            progress.group_snapshots = 1;
            if let Some(info) = snapshot_list.pop() {
                let rel_path =
                    print_ns_and_snapshot(info.backup_dir.backup_ns(), info.backup_dir.as_ref());
                if pool_writer.contains_snapshot(
                    datastore_name,
                    info.backup_dir.backup_ns(),
                    info.backup_dir.as_ref(),
                ) {
                    task_log!(worker, "skip snapshot {}", rel_path);
                    continue;
                }

                need_catalog = true;

                match backup_snapshot(worker, &mut pool_writer, datastore.clone(), info.backup_dir)?
                {
                    SnapshotBackupResult::Success => summary.snapshot_list.push(rel_path),
                    SnapshotBackupResult::Error => errors = true,
                    SnapshotBackupResult::Ignored => {}
                }
                progress.done_snapshots = 1;
                task_log!(worker, "percentage done: {}", progress);
            }
        } else {
            progress.group_snapshots = snapshot_list.len() as u64;
            for (snapshot_number, info) in snapshot_list.into_iter().enumerate() {
                let rel_path =
                    print_ns_and_snapshot(info.backup_dir.backup_ns(), info.backup_dir.as_ref());

                if pool_writer.contains_snapshot(
                    datastore_name,
                    info.backup_dir.backup_ns(),
                    info.backup_dir.as_ref(),
                ) {
                    task_log!(worker, "skip snapshot {}", rel_path);
                    continue;
                }

                need_catalog = true;

                match backup_snapshot(worker, &mut pool_writer, datastore.clone(), info.backup_dir)?
                {
                    SnapshotBackupResult::Success => summary.snapshot_list.push(rel_path),
                    SnapshotBackupResult::Error => errors = true,
                    SnapshotBackupResult::Ignored => {}
                }
                progress.done_snapshots = snapshot_number as u64 + 1;
                task_log!(worker, "percentage done: {}", progress);
            }
        }
    }

    pool_writer.commit()?;

    if need_catalog {
        task_log!(worker, "append media catalog");

        let uuid = pool_writer.load_writable_media(worker)?;
        let done = pool_writer.append_catalog_archive(worker)?;
        if !done {
            task_log!(
                worker,
                "catalog does not fit on tape, writing to next volume"
            );
            pool_writer.set_media_status_full(&uuid)?;
            pool_writer.load_writable_media(worker)?;
            let done = pool_writer.append_catalog_archive(worker)?;
            if !done {
                bail!("write_catalog_archive failed on second media");
            }
        }
    }

    if setup.export_media_set.unwrap_or(false) {
        pool_writer.export_media_set(worker)?;
    } else if setup.eject_media.unwrap_or(false) {
        pool_writer.eject_media(worker)?;
    }

    if errors {
        bail!("Tape backup finished with some errors. Please check the task log.");
    }

    summary.used_tapes = match pool_writer.get_used_media_labels() {
        Ok(tapes) => Some(tapes),
        Err(err) => {
            task_warn!(worker, "could not collect list of used tapes: {err}");
            None
        }
    };

    summary.duration = start.elapsed();

    Ok(())
}

// Try to update the the media online status
fn update_media_online_status(drive: &str) -> Result<Option<String>, Error> {
    let (config, _digest) = pbs_config::drive::config()?;

    if let Ok(Some((mut changer, changer_name))) = media_changer(&config, drive) {
        let label_text_list = changer.online_media_label_texts()?;

        let mut inventory = Inventory::load(TAPE_STATUS_DIR)?;

        update_changer_online_status(&config, &mut inventory, &changer_name, &label_text_list)?;

        Ok(Some(changer_name))
    } else {
        Ok(None)
    }
}

fn backup_snapshot(
    worker: &WorkerTask,
    pool_writer: &mut PoolWriter,
    datastore: Arc<DataStore>,
    snapshot: BackupDir,
) -> Result<SnapshotBackupResult, Error> {
    let snapshot_path = snapshot.relative_path();
    task_log!(worker, "backup snapshot {:?}", snapshot_path);

    let snapshot_reader = match snapshot.locked_reader() {
        Ok(reader) => reader,
        Err(err) => {
            if !snapshot.full_path().exists() {
                // we got an error and the dir does not exist,
                // it probably just vanished, so continue
                task_log!(worker, "snapshot {:?} vanished, skipping", snapshot_path);
                return Ok(SnapshotBackupResult::Ignored);
            }
            task_warn!(
                worker,
                "failed opening snapshot {:?}: {}",
                snapshot_path,
                err
            );
            return Ok(SnapshotBackupResult::Error);
        }
    };

    let snapshot_reader = Arc::new(Mutex::new(snapshot_reader));

    let (reader_thread, chunk_iter) =
        pool_writer.spawn_chunk_reader_thread(datastore.clone(), snapshot_reader.clone())?;

    let mut chunk_iter = chunk_iter.peekable();

    loop {
        worker.check_abort()?;

        // test is we have remaining chunks
        match chunk_iter.peek() {
            None => break,
            Some(Ok(_)) => { /* Ok */ }
            Some(Err(err)) => bail!("{}", err),
        }

        let uuid = pool_writer.load_writable_media(worker)?;

        worker.check_abort()?;

        let (leom, _bytes) =
            pool_writer.append_chunk_archive(worker, &mut chunk_iter, datastore.name())?;

        if leom {
            pool_writer.set_media_status_full(&uuid)?;
        }
    }

    if reader_thread.join().is_err() {
        bail!("chunk reader thread failed");
    }

    worker.check_abort()?;

    let uuid = pool_writer.load_writable_media(worker)?;

    worker.check_abort()?;

    let snapshot_reader = snapshot_reader.lock().unwrap();

    let (done, _bytes) = pool_writer.append_snapshot_archive(worker, &snapshot_reader)?;

    if !done {
        // does not fit on tape, so we try on next volume
        pool_writer.set_media_status_full(&uuid)?;

        worker.check_abort()?;

        pool_writer.load_writable_media(worker)?;
        let (done, _bytes) = pool_writer.append_snapshot_archive(worker, &snapshot_reader)?;

        if !done {
            bail!("write_snapshot_archive failed on second media");
        }
    }

    task_log!(
        worker,
        "end backup {}:{:?}",
        datastore.name(),
        snapshot_path
    );

    Ok(SnapshotBackupResult::Success)
}
