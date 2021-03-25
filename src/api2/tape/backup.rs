use std::path::Path;
use std::sync::{Mutex, Arc};

use anyhow::{bail, format_err, Error};
use serde_json::Value;

use proxmox::{
    try_block,
    api::{
        api,
        RpcEnvironment,
        RpcEnvironmentType,
        Router,
        Permission,
    },
};

use crate::{
    task_log,
    task_warn,
    config::{
        self,
        cached_user_info::CachedUserInfo,
        acl::{
            PRIV_DATASTORE_READ,
            PRIV_TAPE_AUDIT,
            PRIV_TAPE_WRITE,
        },
        tape_job::{
            TapeBackupJobConfig,
            TapeBackupJobSetup,
            TapeBackupJobStatus,
        },
    },
    server::{
        lookup_user_email,
        TapeBackupJobSummary,
        jobstate::{
            Job,
            JobState,
            compute_schedule_status,
        },
    },
    backup::{
        DataStore,
        BackupDir,
        BackupInfo,
        StoreProgress,
    },
    api2::types::{
        Authid,
        UPID_SCHEMA,
        JOB_ID_SCHEMA,
        MediaPoolConfig,
        Userid,
    },
    server::WorkerTask,
    task::TaskState,
    tape::{
        TAPE_STATUS_DIR,
        Inventory,
        PoolWriter,
        MediaPool,
        SnapshotReader,
        drive::{
            media_changer,
            lock_tape_device,
            set_tape_device_state,
        },
        changer::update_changer_online_status,
    },
};

const TAPE_BACKUP_JOB_ROUTER: Router = Router::new()
    .post(&API_METHOD_RUN_TAPE_BACKUP_JOB);

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

    let privs = user_info.lookup_privs(auth_id, &["datastore", store]);
    if (privs & PRIV_DATASTORE_READ) == 0 {
        bail!("no permissions on /datastore/{}", store);
    }

    let privs = user_info.lookup_privs(auth_id, &["tape", "drive", drive]);
    if (privs & PRIV_TAPE_WRITE) == 0 {
        bail!("no permissions on /tape/drive/{}", drive);
    }

    let privs = user_info.lookup_privs(auth_id, &["tape", "pool", pool]);
    if (privs & PRIV_TAPE_WRITE) == 0 {
        bail!("no permissions on /tape/pool/{}", pool);
    }

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
    mut rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<TapeBackupJobStatus>, Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let user_info = CachedUserInfo::new()?;

    let (config, digest) = config::tape_job::config()?;

    let job_list_iter = config
        .convert_to_typed_array("backup")?
        .into_iter()
        .filter(|_job: &TapeBackupJobConfig| {
            // fixme: check access permission
            true
        });

    let mut list = Vec::new();

    for job in job_list_iter {
        let privs = user_info.lookup_privs(&auth_id, &["tape", "job", &job.id]);
        if (privs & PRIV_TAPE_AUDIT) == 0 {
            continue;
        }

        let last_state = JobState::load("tape-backup-job", &job.id)
            .map_err(|err| format_err!("could not open statefile for {}: {}", &job.id, err))?;

        let status = compute_schedule_status(&last_state, job.schedule.as_deref())?;

        list.push(TapeBackupJobStatus { config: job, status });
    }

    rpcenv["digest"] = proxmox::tools::digest_to_hex(&digest).into();

    Ok(list)
}

pub fn do_tape_backup_job(
    mut job: Job,
    setup: TapeBackupJobSetup,
    auth_id: &Authid,
    schedule: Option<String>,
) -> Result<String, Error> {

    let job_id = format!("{}:{}:{}:{}",
                         setup.store,
                         setup.pool,
                         setup.drive,
                         job.jobname());

    let worker_type = job.jobtype().to_string();

    let datastore = DataStore::lookup_datastore(&setup.store)?;

    let (config, _digest) = config::media_pool::config()?;
    let pool_config: MediaPoolConfig = config.lookup("pool", &setup.pool)?;

    let (drive_config, _digest) = config::drive::config()?;

    // for scheduled jobs we acquire the lock later in the worker
    let drive_lock = if schedule.is_some() {
        None
    } else {
        Some(lock_tape_device(&drive_config, &setup.drive)?)
    };

    let notify_user = setup.notify_user.as_ref().unwrap_or_else(|| &Userid::root_userid());
    let email = lookup_user_email(notify_user);

    let upid_str = WorkerTask::new_thread(
        &worker_type,
        Some(job_id.clone()),
        auth_id.clone(),
        false,
        move |worker| {
            job.start(&worker.upid().to_string())?;
            let mut drive_lock = drive_lock;

            let (job_result, summary) = match try_block!({
                if schedule.is_some() {
                    // for scheduled tape backup jobs, we wait indefinitely for the lock
                    task_log!(worker, "waiting for drive lock...");
                    loop {
                        if let Ok(lock) = lock_tape_device(&drive_config, &setup.drive) {
                            drive_lock = Some(lock);
                            break;
                        } // ignore errors

                        worker.check_abort()?;
                    }
                }
                set_tape_device_state(&setup.drive, &worker.upid().to_string())?;

                task_log!(worker,"Starting tape backup job '{}'", job_id);
                if let Some(event_str) = schedule {
                    task_log!(worker,"task triggered by schedule '{}'", event_str);
                }

                backup_worker(
                    &worker,
                    datastore,
                    &pool_config,
                    &setup,
                    email.clone(),
                )
            }) {
                Ok(summary) => (Ok(()), summary),
                Err(err) => (Err(err), Default::default()),
            };

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
                eprintln!(
                    "could not finish job state for {}: {}",
                    job.jobtype().to_string(),
                    err
                );
            }

            if let Err(err) = set_tape_device_state(&setup.drive, "") {
                eprintln!(
                    "could not unset drive state for {}: {}",
                    setup.drive,
                    err
                );
            }

            job_result
        }
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
pub fn run_tape_backup_job(
    id: String,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<String, Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;

    let (config, _digest) = config::tape_job::config()?;
    let backup_job: TapeBackupJobConfig = config.lookup("backup", &id)?;

    check_backup_permission(
        &auth_id,
        &backup_job.setup.store,
        &backup_job.setup.pool,
        &backup_job.setup.drive,
    )?;

    let job = Job::new("tape-backup-job", &id)?;

    let upid_str = do_tape_backup_job(job, backup_job.setup, &auth_id, None)?;

    Ok(upid_str)
}

#[api(
    input: {
        properties: {
            setup: {
                type: TapeBackupJobSetup,
                flatten: true,
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
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;

    check_backup_permission(
        &auth_id,
        &setup.store,
        &setup.pool,
        &setup.drive,
    )?;

    let datastore = DataStore::lookup_datastore(&setup.store)?;

    let (config, _digest) = config::media_pool::config()?;
    let pool_config: MediaPoolConfig = config.lookup("pool", &setup.pool)?;

    let (drive_config, _digest) = config::drive::config()?;

    // early check/lock before starting worker
    let drive_lock = lock_tape_device(&drive_config, &setup.drive)?;

    let to_stdout = rpcenv.env_type() == RpcEnvironmentType::CLI;

    let job_id = format!("{}:{}:{}", setup.store, setup.pool, setup.drive);

    let notify_user = setup.notify_user.as_ref().unwrap_or_else(|| &Userid::root_userid());
    let email = lookup_user_email(notify_user);

    let upid_str = WorkerTask::new_thread(
        "tape-backup",
        Some(job_id),
        auth_id,
        to_stdout,
        move |worker| {
            let _drive_lock = drive_lock; // keep lock guard
            set_tape_device_state(&setup.drive, &worker.upid().to_string())?;

            let (job_result, summary) = match backup_worker(
                &worker,
                datastore,
                &pool_config,
                &setup,
                email.clone(),
            ) {
                Ok(summary) => (Ok(()), summary),
                Err(err) => (Err(err), Default::default()),
            };

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
        }
    )?;

    Ok(upid_str.into())
}

fn backup_worker(
    worker: &WorkerTask,
    datastore: Arc<DataStore>,
    pool_config: &MediaPoolConfig,
    setup: &TapeBackupJobSetup,
    email: Option<String>,
) -> Result<TapeBackupJobSummary, Error> {

    let status_path = Path::new(TAPE_STATUS_DIR);
    let start = std::time::Instant::now();
    let mut summary: TapeBackupJobSummary = Default::default();

    task_log!(worker, "update media online status");
    let changer_name = update_media_online_status(&setup.drive)?;

    let pool = MediaPool::with_config(status_path, &pool_config, changer_name, false)?;

    let mut pool_writer = PoolWriter::new(pool, &setup.drive, worker, email)?;

    let mut group_list = BackupInfo::list_backup_groups(&datastore.base_path())?;

    group_list.sort_unstable();

    let group_count = group_list.len();
    task_log!(worker, "found {} groups", group_count);

    let mut progress = StoreProgress::new(group_count as u64);

    let latest_only = setup.latest_only.unwrap_or(false);

    if latest_only {
        task_log!(worker, "latest-only: true (only considering latest snapshots)");
    }

    let datastore_name = datastore.name();

    let mut errors = false;

    let mut need_catalog = false; // avoid writing catalog for empty jobs

    for (group_number, group) in group_list.into_iter().enumerate() {
        progress.done_groups = group_number as u64;
        progress.done_snapshots = 0;
        progress.group_snapshots = 0;

        let mut snapshot_list = group.list_backups(&datastore.base_path())?;

        BackupInfo::sort_list(&mut snapshot_list, true); // oldest first

        if latest_only {
            progress.group_snapshots = 1;
            if let Some(info) = snapshot_list.pop() {
                if pool_writer.contains_snapshot(datastore_name, &info.backup_dir.to_string()) {
                    task_log!(worker, "skip snapshot {}", info.backup_dir);
                    continue;
                }

                need_catalog = true;

                let snapshot_name = info.backup_dir.to_string();
                if !backup_snapshot(worker, &mut pool_writer, datastore.clone(), info.backup_dir)? {
                    errors = true;
                } else {
                    summary.snapshot_list.push(snapshot_name);
                }
                progress.done_snapshots = 1;
                task_log!(
                    worker,
                    "percentage done: {}",
                    progress
                );
            }
        } else {
            progress.group_snapshots = snapshot_list.len() as u64;
            for (snapshot_number, info) in snapshot_list.into_iter().enumerate() {
                if pool_writer.contains_snapshot(datastore_name, &info.backup_dir.to_string()) {
                    task_log!(worker, "skip snapshot {}", info.backup_dir);
                    continue;
                }

                need_catalog = true;

                let snapshot_name = info.backup_dir.to_string();
                if !backup_snapshot(worker, &mut pool_writer, datastore.clone(), info.backup_dir)? {
                    errors = true;
                } else {
                    summary.snapshot_list.push(snapshot_name);
                }
                progress.done_snapshots = snapshot_number as u64 + 1;
                task_log!(
                    worker,
                    "percentage done: {}",
                    progress
                );
            }
        }
    }

    pool_writer.commit()?;

    if need_catalog {
        task_log!(worker, "append media catalog");

        let uuid = pool_writer.load_writable_media(worker)?;
        let done = pool_writer.append_catalog_archive(worker)?;
        if !done {
            task_log!(worker, "catalog does not fit on tape, writing to next volume");
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

    summary.duration = start.elapsed();

    Ok(summary)
}

// Try to update the the media online status
fn update_media_online_status(drive: &str) -> Result<Option<String>, Error> {

    let (config, _digest) = config::drive::config()?;

    if let Ok(Some((mut changer, changer_name))) = media_changer(&config, drive) {

         let label_text_list = changer.online_media_label_texts()?;

        let status_path = Path::new(TAPE_STATUS_DIR);
        let mut inventory = Inventory::load(status_path)?;

        update_changer_online_status(
            &config,
            &mut inventory,
            &changer_name,
            &label_text_list,
        )?;

        Ok(Some(changer_name))
    } else {
        Ok(None)
    }
}

pub fn backup_snapshot(
    worker: &WorkerTask,
    pool_writer: &mut PoolWriter,
    datastore: Arc<DataStore>,
    snapshot: BackupDir,
) -> Result<bool, Error> {

    task_log!(worker, "backup snapshot {}", snapshot);

    let snapshot_reader = match SnapshotReader::new(datastore.clone(), snapshot.clone()) {
        Ok(reader) => reader,
        Err(err) => {
            // ignore missing snapshots and continue
            task_warn!(worker, "failed opening snapshot '{}': {}", snapshot, err);
            return Ok(false);
        }
    };

    let snapshot_reader = Arc::new(Mutex::new(snapshot_reader));

    let (reader_thread, chunk_iter) = pool_writer.spawn_chunk_reader_thread(
        datastore.clone(),
        snapshot_reader.clone(),
    )?;

    let mut chunk_iter = chunk_iter.peekable();

    loop {
        worker.check_abort()?;

        // test is we have remaining chunks
        match chunk_iter.peek() {
            None => break,
            Some(Ok(_)) => { /* Ok */ },
            Some(Err(err)) => bail!("{}", err),
        }

        let uuid = pool_writer.load_writable_media(worker)?;

        worker.check_abort()?;

        let (leom, _bytes) = pool_writer.append_chunk_archive(worker, &mut chunk_iter, datastore.name())?;

        if leom {
            pool_writer.set_media_status_full(&uuid)?;
        }
    }

    if let Err(_) = reader_thread.join() {
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

    task_log!(worker, "end backup {}:{}", datastore.name(), snapshot);

    Ok(true)
}
