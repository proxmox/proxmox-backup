use std::path::Path;
use std::sync::Arc;

use anyhow::{bail, Error};
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
    config::{
        self,
        drive::check_drive_exists,
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
        UPID_SCHEMA,
        MediaPoolConfig,
    },
    server::WorkerTask,
    tape::{
        TAPE_STATUS_DIR,
        Inventory,
        PoolWriter,
        MediaPool,
        SnapshotReader,
        media_changer,
        update_changer_online_status,
    },
};

#[api(
   input: {
        properties: {
            store: {
                schema: DATASTORE_SCHEMA,
            },
            pool: {
                schema: MEDIA_POOL_NAME_SCHEMA,
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
    eject_media: Option<bool>,
    export_media_set: Option<bool>,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;

    let datastore = DataStore::lookup_datastore(&store)?;

    let (config, _digest) = config::media_pool::config()?;
    let pool_config: MediaPoolConfig = config.lookup("pool", &pool)?;

    let (drive_config, _digest) = config::drive::config()?;
    // early check before starting worker
    check_drive_exists(&drive_config, &pool_config.drive)?;

    let to_stdout = if rpcenv.env_type() == RpcEnvironmentType::CLI { true } else { false };

    let eject_media = eject_media.unwrap_or(false);
    let export_media_set = export_media_set.unwrap_or(false);

    let upid_str = WorkerTask::new_thread(
        "tape-backup",
        Some(store.clone()),
        auth_id,
        to_stdout,
        move |worker| {
            backup_worker(&worker, datastore, &pool_config, eject_media, export_media_set)?;
            Ok(())
        }
    )?;

    Ok(upid_str.into())
}

pub const ROUTER: Router = Router::new()
    .post(&API_METHOD_BACKUP);


fn backup_worker(
    worker: &WorkerTask,
    datastore: Arc<DataStore>,
    pool_config: &MediaPoolConfig,
    eject_media: bool,
    export_media_set: bool,
) -> Result<(), Error> {

    let status_path = Path::new(TAPE_STATUS_DIR);

    let _lock = MediaPool::lock(status_path, &pool_config.name)?;

    worker.log("update media online status");
    let has_changer = update_media_online_status(&pool_config.drive)?;

    let use_offline_media = !has_changer;

    let pool = MediaPool::with_config(status_path, &pool_config, use_offline_media)?;

    let mut pool_writer = PoolWriter::new(pool, &pool_config.drive)?;

    let mut group_list = BackupInfo::list_backup_groups(&datastore.base_path())?;

    group_list.sort_unstable();

    for group in group_list {
        let mut snapshot_list = group.list_backups(&datastore.base_path())?;
        BackupInfo::sort_list(&mut snapshot_list, true); // oldest first

        for info in snapshot_list {
            if pool_writer.contains_snapshot(&info.backup_dir.to_string()) {
                continue;
            }
            worker.log(format!("backup snapshot {}", info.backup_dir));
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
fn update_media_online_status(drive: &str) -> Result<bool, Error> {

    let (config, _digest) = config::drive::config()?;

    let mut has_changer = false;

    if let Ok(Some((mut changer, changer_name))) = media_changer(&config, drive) {

        has_changer = true;

        let changer_id_list = changer.online_media_changer_ids()?;

        let status_path = Path::new(TAPE_STATUS_DIR);
        let mut inventory = Inventory::load(status_path)?;

        update_changer_online_status(
            &config,
            &mut inventory,
            &changer_name,
            &changer_id_list,
        )?;
    }

    Ok(has_changer)
}

pub fn backup_snapshot(
    worker: &WorkerTask,
    pool_writer: &mut PoolWriter,
    datastore: Arc<DataStore>,
    snapshot: BackupDir,
) -> Result<(), Error> {

    worker.log(format!("start backup {}:{}", datastore.name(), snapshot));

    let snapshot_reader = SnapshotReader::new(datastore.clone(), snapshot.clone())?;

    let mut chunk_iter = snapshot_reader.chunk_iterator()?.peekable();

    loop {
        // test is we have remaining chunks
        if chunk_iter.peek().is_none() {
            break;
        }

        let uuid = pool_writer.load_writable_media(worker)?;

        let (leom, _bytes) = pool_writer.append_chunk_archive(worker, &datastore, &mut chunk_iter)?;

        if leom {
            pool_writer.set_media_status_full(&uuid)?;
        }
    }

    let uuid = pool_writer.load_writable_media(worker)?;

    let (done, _bytes) = pool_writer.append_snapshot_archive(worker, &snapshot_reader)?;

    if !done {
        // does not fit on tape, so we try on next volume
        pool_writer.set_media_status_full(&uuid)?;

        pool_writer.load_writable_media(worker)?;
        let (done, _bytes) = pool_writer.append_snapshot_archive(worker, &snapshot_reader)?;

        if !done {
            bail!("write_snapshot_archive failed on second media");
        }
    }

    worker.log(format!("end backup {}:{}", datastore.name(), snapshot));

    Ok(())
}
