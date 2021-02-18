use std::path::Path;
use std::sync::Arc;

use anyhow::{bail, format_err, Error};
use serde_json::Value;

use proxmox::{
    api::{
        api,
        RpcEnvironment,
        RpcEnvironmentType,
        Router,
    },
};

use crate::{
    task_log,
    config::{
        self,
        tape_job::{
            TapeBackupJobConfig,
            TapeBackupJobStatus,
        },
    },
    server::{
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
    },
    api2::types::{
        Authid,
        DATASTORE_SCHEMA,
        MEDIA_POOL_NAME_SCHEMA,
        DRIVE_NAME_SCHEMA,
        UPID_SCHEMA,
        JOB_ID_SCHEMA,
        MediaPoolConfig,
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

#[api(
    returns: {
        description: "List configured thape backup jobs and their status",
        type: Array,
        items: { type: TapeBackupJobStatus },
    },
)]
/// List all tape backup jobs
pub fn list_tape_backup_jobs(
    _param: Value,
    mut rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<TapeBackupJobStatus>, Error> {

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
    tape_job: TapeBackupJobConfig,
    auth_id: &Authid,
    schedule: Option<String>,
) -> Result<String, Error> {

    let job_id = format!("{}:{}:{}:{}",
                         tape_job.store,
                         tape_job.pool,
                         tape_job.drive,
                         job.jobname());

    let worker_type = job.jobtype().to_string();

    let datastore = DataStore::lookup_datastore(&tape_job.store)?;

    let (config, _digest) = config::media_pool::config()?;
    let pool_config: MediaPoolConfig = config.lookup("pool", &tape_job.pool)?;

    let (drive_config, _digest) = config::drive::config()?;

    // early check/lock before starting worker
    let drive_lock = lock_tape_device(&drive_config, &tape_job.drive)?;

    let upid_str = WorkerTask::new_thread(
        &worker_type,
        Some(job_id.clone()),
        auth_id.clone(),
        false,
        move |worker| {
            let _drive_lock = drive_lock; // keep lock guard

            set_tape_device_state(&tape_job.drive, &worker.upid().to_string())?;
            job.start(&worker.upid().to_string())?;

            let eject_media = false;
            let export_media_set = false;

            task_log!(worker,"Starting tape backup job '{}'", job_id);
            if let Some(event_str) = schedule {
                task_log!(worker,"task triggered by schedule '{}'", event_str);
            }

            let job_result = backup_worker(
                &worker,
                datastore,
                &tape_job.drive,
                &pool_config,
                eject_media,
                export_media_set,
            );

            let status = worker.create_state(&job_result);

            if let Err(err) = job.finish(status) {
                eprintln!(
                    "could not finish job state for {}: {}",
                    job.jobtype().to_string(),
                    err
                );
            }

            if let Err(err) = set_tape_device_state(&tape_job.drive, "") {
                eprintln!(
                    "could not unset drive state for {}: {}",
                    tape_job.drive,
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
)]
/// Runs a tape backup job manually.
pub fn run_tape_backup_job(
    id: String,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<String, Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;

    let (config, _digest) = config::tape_job::config()?;
    let backup_job: TapeBackupJobConfig = config.lookup("backup", &id)?;

    let job = Job::new("tape-backup-job", &id)?;

    let upid_str = do_tape_backup_job(job, backup_job, &auth_id, None)?;

    Ok(upid_str)
}

#[api(
   input: {
        properties: {
            store: {
                schema: DATASTORE_SCHEMA,
            },
            pool: {
                schema: MEDIA_POOL_NAME_SCHEMA,
            },
            drive: {
                schema: DRIVE_NAME_SCHEMA,
            },
            "eject-media": {
                description: "Eject media upon job completion.",
                type: bool,
                optional: true,
            },
            "export-media-set": {
                description: "Export media set upon job completion.",
                type: bool,
                optional: true,
            },
        },
    },
    returns: {
        schema: UPID_SCHEMA,
    },
)]
/// Backup datastore to tape media pool
pub fn backup(
    store: String,
    pool: String,
    drive: String,
    eject_media: Option<bool>,
    export_media_set: Option<bool>,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;

    let datastore = DataStore::lookup_datastore(&store)?;

    let (config, _digest) = config::media_pool::config()?;
    let pool_config: MediaPoolConfig = config.lookup("pool", &pool)?;

    let (drive_config, _digest) = config::drive::config()?;

    // early check/lock before starting worker
    let drive_lock = lock_tape_device(&drive_config, &drive)?;

    let to_stdout = rpcenv.env_type() == RpcEnvironmentType::CLI;

    let eject_media = eject_media.unwrap_or(false);
    let export_media_set = export_media_set.unwrap_or(false);

    let job_id = format!("{}:{}:{}", store, pool, drive);

    let upid_str = WorkerTask::new_thread(
        "tape-backup",
        Some(job_id),
        auth_id,
        to_stdout,
        move |worker| {
            let _drive_lock = drive_lock; // keep lock guard
            set_tape_device_state(&drive, &worker.upid().to_string())?;
            backup_worker(&worker, datastore, &drive, &pool_config, eject_media, export_media_set)?;
            // ignore errors
            let _ = set_tape_device_state(&drive, "");
            Ok(())
        }
    )?;

    Ok(upid_str.into())
}

fn backup_worker(
    worker: &WorkerTask,
    datastore: Arc<DataStore>,
    drive: &str,
    pool_config: &MediaPoolConfig,
    eject_media: bool,
    export_media_set: bool,
) -> Result<(), Error> {

    let status_path = Path::new(TAPE_STATUS_DIR);

    let _lock = MediaPool::lock(status_path, &pool_config.name)?;

    task_log!(worker, "update media online status");
    let changer_name = update_media_online_status(drive)?;

    let pool = MediaPool::with_config(status_path, &pool_config, changer_name)?;

    let mut pool_writer = PoolWriter::new(pool, drive)?;

    let mut group_list = BackupInfo::list_backup_groups(&datastore.base_path())?;

    group_list.sort_unstable();

    for group in group_list {
        let mut snapshot_list = group.list_backups(&datastore.base_path())?;
        BackupInfo::sort_list(&mut snapshot_list, true); // oldest first

        for info in snapshot_list {
            if pool_writer.contains_snapshot(&info.backup_dir.to_string()) {
                continue;
            }
            task_log!(worker, "backup snapshot {}", info.backup_dir);
            backup_snapshot(worker, &mut pool_writer, datastore.clone(), info.backup_dir)?;
        }
    }

    pool_writer.commit()?;

    if export_media_set {
        pool_writer.export_media_set(worker)?;
    } else if eject_media {
        pool_writer.eject_media(worker)?;
    }

    Ok(())
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
) -> Result<(), Error> {

    task_log!(worker, "start backup {}:{}", datastore.name(), snapshot);

    let snapshot_reader = SnapshotReader::new(datastore.clone(), snapshot.clone())?;

    let mut chunk_iter = snapshot_reader.chunk_iterator()?.peekable();

    loop {
        worker.check_abort()?;

        // test is we have remaining chunks
        if chunk_iter.peek().is_none() {
            break;
        }

        let uuid = pool_writer.load_writable_media(worker)?;

        worker.check_abort()?;

        let (leom, _bytes) = pool_writer.append_chunk_archive(worker, &datastore, &mut chunk_iter)?;

        if leom {
            pool_writer.set_media_status_full(&uuid)?;
        }
    }

    worker.check_abort()?;

    let uuid = pool_writer.load_writable_media(worker)?;

    worker.check_abort()?;

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

    Ok(())
}
