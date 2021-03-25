use std::panic::UnwindSafe;
use std::path::Path;
use std::sync::Arc;

use anyhow::{bail, format_err, Error};
use serde_json::Value;

use proxmox::{
    sortable,
    identity,
    list_subdirs_api_method,
    tools::Uuid,
    sys::error::SysError,
    api::{
        api,
        section_config::SectionConfigData,
        RpcEnvironment,
        RpcEnvironmentType,
        Permission,
        Router,
        SubdirMap,
    },
};

use crate::{
    task_log,
    config::{
        self,
        cached_user_info::CachedUserInfo,
        acl::{
            PRIV_TAPE_AUDIT,
            PRIV_TAPE_READ,
            PRIV_TAPE_WRITE,
        },
    },
    api2::{
        types::{
            UPID_SCHEMA,
            CHANGER_NAME_SCHEMA,
            DRIVE_NAME_SCHEMA,
            MEDIA_LABEL_SCHEMA,
            MEDIA_POOL_NAME_SCHEMA,
            Authid,
            DriveListEntry,
            LinuxTapeDrive,
            MediaIdFlat,
            LabelUuidMap,
            MamAttribute,
            LinuxDriveAndMediaStatus,
        },
        tape::restore::{
            fast_catalog_restore,
            restore_media,
        },
    },
    server::WorkerTask,
    tape::{
        TAPE_STATUS_DIR,
        Inventory,
        MediaCatalog,
        MediaId,
        lock_media_set,
        lock_media_pool,
        lock_unassigned_media_pool,
        linux_tape_device_list,
        lookup_device_identification,
        file_formats::{
            MediaLabel,
            MediaSetLabel,
        },
        drive::{
            TapeDriver,
            LinuxTapeHandle,
            Lp17VolumeStatistics,
            open_linux_tape_device,
            media_changer,
            required_media_changer,
            open_drive,
            lock_tape_device,
            set_tape_device_state,
            get_tape_device_state,
            tape_alert_flags_critical,
        },
        changer::update_changer_online_status,
    },
};

fn run_drive_worker<F>(
    rpcenv: &dyn RpcEnvironment,
    drive: String,
    worker_type: &str,
    job_id: Option<String>,
    f: F,
) -> Result<String, Error>
where
    F: Send
        + UnwindSafe
        + 'static
        + FnOnce(Arc<WorkerTask>, SectionConfigData) -> Result<(), Error>,
{
    // early check/lock before starting worker
    let (config, _digest) = config::drive::config()?;
    let lock_guard = lock_tape_device(&config, &drive)?;

    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let to_stdout = rpcenv.env_type() == RpcEnvironmentType::CLI;

    WorkerTask::new_thread(worker_type, job_id, auth_id, to_stdout, move |worker| {
        let _lock_guard = lock_guard;
        set_tape_device_state(&drive, &worker.upid().to_string())
            .map_err(|err| format_err!("could not set tape device state: {}", err))?;

        let result = f(worker, config);
        set_tape_device_state(&drive, "")
            .map_err(|err| format_err!("could not unset tape device state: {}", err))?;
        result
    })
}

async fn run_drive_blocking_task<F, R>(drive: String, state: String, f: F) -> Result<R, Error>
where
    F: Send + 'static + FnOnce(SectionConfigData) -> Result<R, Error>,
    R: Send + 'static,
{
    // early check/lock before starting worker
    let (config, _digest) = config::drive::config()?;
    let lock_guard = lock_tape_device(&config, &drive)?;
    tokio::task::spawn_blocking(move || {
        let _lock_guard = lock_guard;
        set_tape_device_state(&drive, &state)
            .map_err(|err| format_err!("could not set tape device state: {}", err))?;
        let result = f(config);
        set_tape_device_state(&drive, "")
            .map_err(|err| format_err!("could not unset tape device state: {}", err))?;
        result
    })
    .await?
}

#[api(
    input: {
        properties: {
            drive: {
                schema: DRIVE_NAME_SCHEMA,
            },
            "label-text": {
                schema: MEDIA_LABEL_SCHEMA,
            },
        },
    },
    returns: {
        schema: UPID_SCHEMA,
    },
    access: {
        permission: &Permission::Privilege(&["tape", "device", "{drive}"], PRIV_TAPE_READ, false),
    },
)]
/// Load media with specified label
///
/// Issue a media load request to the associated changer device.
pub fn load_media(
    drive: String,
    label_text: String,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {
    let job_id = format!("{}:{}", drive, label_text);

    let upid_str = run_drive_worker(
        rpcenv,
        drive.clone(),
        "load-media",
        Some(job_id),
        move |worker, config| {
            task_log!(worker, "loading media '{}' into drive '{}'", label_text, drive);
            let (mut changer, _) = required_media_changer(&config, &drive)?;
            changer.load_media(&label_text)?;
            Ok(())
        },
    )?;

    Ok(upid_str.into())
}

#[api(
    input: {
        properties: {
            drive: {
                schema: DRIVE_NAME_SCHEMA,
            },
            "source-slot": {
                description: "Source slot number.",
                minimum: 1,
            },
        },
    },
    access: {
        permission: &Permission::Privilege(&["tape", "device", "{drive}"], PRIV_TAPE_READ, false),
    },
)]
/// Load media from the specified slot
///
/// Issue a media load request to the associated changer device.
pub async fn load_slot(drive: String, source_slot: u64) -> Result<(), Error> {
    run_drive_blocking_task(
        drive.clone(),
        format!("load from slot {}", source_slot),
        move |config| {
            let (mut changer, _) = required_media_changer(&config, &drive)?;
            changer.load_media_from_slot(source_slot)?;
            Ok(())
        },
    )
    .await
}

#[api(
    input: {
        properties: {
            drive: {
                schema: DRIVE_NAME_SCHEMA,
            },
            "label-text": {
                schema: MEDIA_LABEL_SCHEMA,
            },
        },
    },
    returns: {
        description: "The import-export slot number the media was transferred to.",
        type: u64,
        minimum: 1,
    },
    access: {
        permission: &Permission::Privilege(&["tape", "device", "{drive}"], PRIV_TAPE_READ, false),
    },
)]
/// Export media with specified label
pub async fn export_media(drive: String, label_text: String) -> Result<u64, Error> {
    run_drive_blocking_task(
        drive.clone(),
        format!("export media {}", label_text),
        move |config| {
            let (mut changer, changer_name) = required_media_changer(&config, &drive)?;
            match changer.export_media(&label_text)? {
                Some(slot) => Ok(slot),
                None => bail!(
                    "media '{}' is not online (via changer '{}')",
                    label_text,
                    changer_name
                ),
            }
        }
    )
    .await
}

#[api(
    input: {
        properties: {
            drive: {
                schema: DRIVE_NAME_SCHEMA,
            },
            "target-slot": {
                description: "Target slot number. If omitted, defaults to the slot that the drive was loaded from.",
                minimum: 1,
                optional: true,
            },
        },
    },
    returns: {
        schema: UPID_SCHEMA,
    },
    access: {
        permission: &Permission::Privilege(&["tape", "device", "{drive}"], PRIV_TAPE_READ, false),
    },
)]
/// Unload media via changer
pub fn unload(
    drive: String,
    target_slot: Option<u64>,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {
    let upid_str = run_drive_worker(
        rpcenv,
        drive.clone(),
        "unload-media",
        Some(drive.clone()),
        move |worker, config| {
            task_log!(worker, "unloading media from drive '{}'", drive);

            let (mut changer, _) = required_media_changer(&config, &drive)?;
            changer.unload_media(target_slot)?;
            Ok(())
        },
    )?;

    Ok(upid_str.into())
}

#[api(
    input: {
        properties: {
            drive: {
                schema: DRIVE_NAME_SCHEMA,
            },
            fast: {
                description: "Use fast erase.",
                type: bool,
                optional: true,
                default: true,
            },
            "label-text": {
                schema: MEDIA_LABEL_SCHEMA,
                optional: true,
            },
        },
    },
    returns: {
        schema: UPID_SCHEMA,
    },
    access: {
        permission: &Permission::Privilege(&["tape", "device", "{drive}"], PRIV_TAPE_WRITE, false),
    },
)]
/// Erase media. Check for label-text if given (cancels if wrong media).
pub fn erase_media(
    drive: String,
    fast: Option<bool>,
    label_text: Option<String>,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {
    let upid_str = run_drive_worker(
        rpcenv,
        drive.clone(),
        "erase-media",
        Some(drive.clone()),
        move |worker, config| {
            if let Some(ref label) = label_text {
                task_log!(worker, "try to load media '{}'", label);
                if let Some((mut changer, _)) = media_changer(&config, &drive)? {
                    changer.load_media(label)?;
                }
            }

            let mut handle = open_drive(&config, &drive)?;

            match handle.read_label() {
                Err(err) => {
                    if let Some(label) = label_text {
                        bail!("expected label '{}', found unrelated data", label);
                    }
                    /* assume drive contains no or unrelated data */
                    task_log!(worker, "unable to read media label: {}", err);
                    task_log!(worker, "erase anyways");
                    handle.erase_media(fast.unwrap_or(true))?;
                }
                Ok((None, _)) => {
                    if let Some(label) = label_text {
                        bail!("expected label '{}', found empty tape", label);
                    }
                    task_log!(worker, "found empty media - erase anyways");
                    handle.erase_media(fast.unwrap_or(true))?;
                }
                Ok((Some(media_id), _key_config)) => {
                    if let Some(label_text) = label_text {
                        if media_id.label.label_text != label_text {
                            bail!(
                                "expected label '{}', found '{}', aborting",
                                label_text,
                                media_id.label.label_text
                            );
                        }
                    }

                    task_log!(
                        worker,
                        "found media '{}' with uuid '{}'",
                        media_id.label.label_text, media_id.label.uuid,
                    );

                    let status_path = Path::new(TAPE_STATUS_DIR);
                    let mut inventory = Inventory::new(status_path);

                    if let Some(MediaSetLabel { ref pool, ref uuid, ..}) =  media_id.media_set_label {
                        let _pool_lock = lock_media_pool(status_path, pool)?;
                        let _media_set_lock = lock_media_set(status_path, uuid, None)?;
                        MediaCatalog::destroy(status_path, &media_id.label.uuid)?;
                        inventory.remove_media(&media_id.label.uuid)?;
                    } else {
                        let _lock = lock_unassigned_media_pool(status_path)?;
                        MediaCatalog::destroy(status_path, &media_id.label.uuid)?;
                        inventory.remove_media(&media_id.label.uuid)?;
                    };

                    handle.erase_media(fast.unwrap_or(true))?;
                }
            }

            Ok(())
        },
    )?;

    Ok(upid_str.into())
}

#[api(
    input: {
        properties: {
            drive: {
                schema: DRIVE_NAME_SCHEMA,
            },
        },
    },
    returns: {
        schema: UPID_SCHEMA,
    },
    access: {
        permission: &Permission::Privilege(&["tape", "device", "{drive}"], PRIV_TAPE_READ, false),
    },
)]
/// Rewind tape
pub fn rewind(
    drive: String,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {
    let upid_str = run_drive_worker(
        rpcenv,
        drive.clone(),
        "rewind-media",
        Some(drive.clone()),
        move |_worker, config| {
            let mut drive = open_drive(&config, &drive)?;
            drive.rewind()?;
            Ok(())
        },
    )?;

    Ok(upid_str.into())
}

#[api(
    input: {
        properties: {
            drive: {
                schema: DRIVE_NAME_SCHEMA,
            },
        },
    },
    returns: {
        schema: UPID_SCHEMA,
    },
    access: {
        permission: &Permission::Privilege(&["tape", "device", "{drive}"], PRIV_TAPE_READ, false),
    },
)]
/// Eject/Unload drive media
pub fn eject_media(
    drive: String,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {
    let upid_str = run_drive_worker(
        rpcenv,
        drive.clone(),
        "eject-media",
        Some(drive.clone()),
        move |_worker, config| {
            if let Some((mut changer, _)) = media_changer(&config, &drive)? {
                changer.unload_media(None)?;
            } else {
                let mut drive = open_drive(&config, &drive)?;
                drive.eject_media()?;
            }
            Ok(())
        },
    )?;

    Ok(upid_str.into())
}

#[api(
    input: {
        properties: {
            drive: {
                schema: DRIVE_NAME_SCHEMA,
            },
            "label-text": {
                schema: MEDIA_LABEL_SCHEMA,
            },
            pool: {
                schema: MEDIA_POOL_NAME_SCHEMA,
                optional: true,
            },
        },
    },
    returns: {
        schema: UPID_SCHEMA,
    },
    access: {
        permission: &Permission::Privilege(&["tape", "device", "{drive}"], PRIV_TAPE_WRITE, false),
    },
)]
/// Label media
///
/// Write a new media label to the media in 'drive'. The media is
/// assigned to the specified 'pool', or else to the free media pool.
///
/// Note: The media need to be empty (you may want to erase it first).
pub fn label_media(
    drive: String,
    pool: Option<String>,
    label_text: String,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {
    if let Some(ref pool) = pool {
        let (pool_config, _digest) = config::media_pool::config()?;

        if pool_config.sections.get(pool).is_none() {
            bail!("no such pool ('{}')", pool);
        }
    }
    let upid_str = run_drive_worker(
        rpcenv,
        drive.clone(),
        "label-media",
        Some(drive.clone()),
        move |worker, config| {
            let mut drive = open_drive(&config, &drive)?;

            drive.rewind()?;

            match drive.read_next_file() {
                Ok(Some(_file)) => bail!("media is not empty (erase first)"),
                Ok(None) => { /* EOF mark at BOT, assume tape is empty */ },
                Err(err) => {
                    if err.is_errno(nix::errno::Errno::ENOSPC) || err.is_errno(nix::errno::Errno::EIO) {
                        /* assume tape is empty */
                    } else {
                        bail!("media read error - {}", err);
                    }
                }
            }

            let ctime = proxmox::tools::time::epoch_i64();
            let label = MediaLabel {
                label_text: label_text.to_string(),
                uuid: Uuid::generate(),
                ctime,
            };

            write_media_label(worker, &mut drive, label, pool)
        },
    )?;

    Ok(upid_str.into())
}

fn write_media_label(
    worker: Arc<WorkerTask>,
    drive: &mut Box<dyn TapeDriver>,
    label: MediaLabel,
    pool: Option<String>,
) -> Result<(), Error> {

    drive.label_tape(&label)?;

    let status_path = Path::new(TAPE_STATUS_DIR);

    let media_id = if let Some(ref pool) = pool {
        // assign media to pool by writing special media set label
        worker.log(format!("Label media '{}' for pool '{}'", label.label_text, pool));
        let set = MediaSetLabel::with_data(&pool, [0u8; 16].into(), 0, label.ctime, None);

        drive.write_media_set_label(&set, None)?;

        let media_id = MediaId { label, media_set_label: Some(set) };

        // Create the media catalog
        MediaCatalog::overwrite(status_path, &media_id, false)?;

        let mut inventory = Inventory::new(status_path);
        inventory.store(media_id.clone(), false)?;

        media_id
    } else {
        worker.log(format!("Label media '{}' (no pool assignment)", label.label_text));

        let media_id = MediaId { label, media_set_label: None };

        // Create the media catalog
        MediaCatalog::overwrite(status_path, &media_id, false)?;

        let mut inventory = Inventory::new(status_path);
        inventory.store(media_id.clone(), false)?;

        media_id
    };

    drive.rewind()?;

    match drive.read_label() {
        Ok((Some(info), _)) => {
            if info.label.uuid != media_id.label.uuid {
                bail!("verify label failed - got wrong label uuid");
            }
            if let Some(ref pool) = pool {
                match info.media_set_label {
                    Some(set) => {
                        if set.uuid != [0u8; 16].into() {
                            bail!("verify media set label failed - got wrong set uuid");
                        }
                        if &set.pool != pool {
                            bail!("verify media set label failed - got wrong pool");
                        }
                    }
                    None => {
                        bail!("verify media set label failed (missing set label)");
                    }
                }
            }
        },
        Ok((None, _)) => bail!("verify label failed (got empty media)"),
        Err(err) => bail!("verify label failed - {}", err),
    };

    drive.rewind()?;

    Ok(())
}

#[api(
    protected: true,
    input: {
        properties: {
            drive: {
                schema: DRIVE_NAME_SCHEMA,
            },
            password: {
                description: "Encryption key password.",
            },
        },
    },
    access: {
        permission: &Permission::Privilege(&["tape", "device", "{drive}"], PRIV_TAPE_READ, false),
    },
)]
/// Try to restore a tape encryption key
pub async fn restore_key(
    drive: String,
    password: String,
) -> Result<(), Error> {
    run_drive_blocking_task(
        drive.clone(),
        "restore key".to_string(),
        move |config| {
            let mut drive = open_drive(&config, &drive)?;

            let (_media_id, key_config) = drive.read_label()?;

            if let Some(key_config) = key_config {
                let password_fn = || { Ok(password.as_bytes().to_vec()) };
                let (key, ..) = key_config.decrypt(&password_fn)?;
                config::tape_encryption_keys::insert_key(key, key_config, true)?;
            } else {
                bail!("media does not contain any encryption key configuration");
            }

            Ok(())
        }
    )
    .await
}

 #[api(
    input: {
        properties: {
            drive: {
                schema: DRIVE_NAME_SCHEMA,
            },
            inventorize: {
                description: "Inventorize media",
                optional: true,
            },
        },
    },
    returns: {
        type: MediaIdFlat,
    },
    access: {
        permission: &Permission::Privilege(&["tape", "device", "{drive}"], PRIV_TAPE_READ, false),
    },
)]
/// Read media label (optionally inventorize media)
pub async fn read_label(
    drive: String,
    inventorize: Option<bool>,
) -> Result<MediaIdFlat, Error> {
    run_drive_blocking_task(
        drive.clone(),
        "reading label".to_string(),
        move |config| {
            let mut drive = open_drive(&config, &drive)?;

            let (media_id, _key_config) = drive.read_label()?;

            let media_id = match media_id {
                Some(media_id) => {
                    let mut flat = MediaIdFlat {
                        uuid: media_id.label.uuid.clone(),
                        label_text: media_id.label.label_text.clone(),
                        ctime: media_id.label.ctime,
                        media_set_ctime: None,
                        media_set_uuid: None,
                        encryption_key_fingerprint: None,
                        pool: None,
                        seq_nr: None,
                    };
                    if let Some(ref set) = media_id.media_set_label {
                        flat.pool = Some(set.pool.clone());
                        flat.seq_nr = Some(set.seq_nr);
                        flat.media_set_uuid = Some(set.uuid.clone());
                        flat.media_set_ctime = Some(set.ctime);
                        flat.encryption_key_fingerprint = set
                            .encryption_key_fingerprint
                            .as_ref()
                            .map(|fp| crate::tools::format::as_fingerprint(fp.bytes()));

                        let encrypt_fingerprint = set.encryption_key_fingerprint.clone()
                            .map(|fp| (fp, set.uuid.clone()));

                        if let Err(err) = drive.set_encryption(encrypt_fingerprint) {
                            // try, but ignore errors. just log to stderr
                            eprintln!("unable to load encryption key: {}", err);
                        }
                    }

                    if let Some(true) = inventorize {
                        let state_path = Path::new(TAPE_STATUS_DIR);
                        let mut inventory = Inventory::new(state_path);

                        if let Some(MediaSetLabel { ref pool, ref uuid, ..}) =  media_id.media_set_label {
                            let _pool_lock = lock_media_pool(state_path, pool)?;
                            let _lock = lock_media_set(state_path, uuid, None)?;
                            MediaCatalog::destroy_unrelated_catalog(state_path, &media_id)?;
                            inventory.store(media_id, false)?;
                        } else {
                            let _lock = lock_unassigned_media_pool(state_path)?;
                            MediaCatalog::destroy(state_path, &media_id.label.uuid)?;
                            inventory.store(media_id, false)?;
                        };
                    }

                    flat
                }
                None => {
                    bail!("Media is empty (no label).");
                }
            };

            Ok(media_id)
        }
    )
    .await
}

#[api(
    input: {
        properties: {
            drive: {
                schema: DRIVE_NAME_SCHEMA,
            },
        },
    },
    returns: {
        schema: UPID_SCHEMA,
    },
    access: {
        permission: &Permission::Privilege(&["tape", "device", "{drive}"], PRIV_TAPE_READ, false),
    },
)]
/// Clean drive
pub fn clean_drive(
    drive: String,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {
    let upid_str = run_drive_worker(
        rpcenv,
        drive.clone(),
        "clean-drive",
        Some(drive.clone()),
        move |worker, config| {
            let (mut changer, _changer_name) = required_media_changer(&config, &drive)?;

            worker.log("Starting drive clean");

            changer.clean_drive()?;

             if let Ok(drive_config) = config.lookup::<LinuxTapeDrive>("linux", &drive) {
                 // Note: clean_drive unloads the cleaning media, so we cannot use drive_config.open
                 let mut handle = LinuxTapeHandle::new(open_linux_tape_device(&drive_config.path)?);

                 // test for critical tape alert flags
                 if let Ok(alert_flags) = handle.tape_alert_flags() {
                     if !alert_flags.is_empty() {
                         worker.log(format!("TapeAlertFlags: {:?}", alert_flags));
                         if tape_alert_flags_critical(alert_flags) {
                             bail!("found critical tape alert flags: {:?}", alert_flags);
                         }
                     }
                 }

                 // test wearout (max. 50 mounts)
                 if let Ok(volume_stats) = handle.volume_statistics() {
                     worker.log(format!("Volume mounts: {}", volume_stats.volume_mounts));
                     let wearout = volume_stats.volume_mounts * 2; // (*100.0/50.0);
                     worker.log(format!("Cleaning tape wearout: {}%", wearout));
                 }
             }

            worker.log("Drive cleaned successfully");

            Ok(())
        },
    )?;

    Ok(upid_str.into())
}

#[api(
    input: {
        properties: {
            drive: {
                schema: DRIVE_NAME_SCHEMA,
            },
        },
    },
    returns: {
        description: "The list of media labels with associated media Uuid (if any).",
        type: Array,
        items: {
            type: LabelUuidMap,
        },
    },
    access: {
        permission: &Permission::Privilege(&["tape", "device", "{drive}"], PRIV_TAPE_READ, false),
    },
)]
/// List known media labels (Changer Inventory)
///
/// Note: Only useful for drives with associated changer device.
///
/// This method queries the changer to get a list of media labels.
///
/// Note: This updates the media online status.
pub async fn inventory(
    drive: String,
) -> Result<Vec<LabelUuidMap>, Error> {
    run_drive_blocking_task(
        drive.clone(),
        "inventorize".to_string(),
        move |config| {
            let (mut changer, changer_name) = required_media_changer(&config, &drive)?;

            let label_text_list = changer.online_media_label_texts()?;

            let state_path = Path::new(TAPE_STATUS_DIR);

            let mut inventory = Inventory::load(state_path)?;

            update_changer_online_status(
                &config,
                &mut inventory,
                &changer_name,
                &label_text_list,
            )?;

            let mut list = Vec::new();

            for label_text in label_text_list.iter() {
                if label_text.starts_with("CLN") {
                    // skip cleaning unit
                    continue;
                }

                let label_text = label_text.to_string();

                if let Some(media_id) = inventory.find_media_by_label_text(&label_text) {
                    list.push(LabelUuidMap { label_text, uuid: Some(media_id.label.uuid.clone()) });
                } else {
                    list.push(LabelUuidMap { label_text, uuid: None });
                }
            }

            Ok(list)
        }
    )
    .await
}

#[api(
    input: {
        properties: {
            drive: {
                schema: DRIVE_NAME_SCHEMA,
            },
            "read-all-labels": {
                description: "Load all tapes and try read labels (even if already inventoried)",
                type: bool,
                optional: true,
            },
        },
    },
    returns: {
        schema: UPID_SCHEMA,
    },
    access: {
        permission: &Permission::Privilege(&["tape", "device", "{drive}"], PRIV_TAPE_READ, false),
    },
)]
/// Update inventory
///
/// Note: Only useful for drives with associated changer device.
///
/// This method queries the changer to get a list of media labels. It
/// then loads any unknown media into the drive, reads the label, and
/// store the result to the media database.
///
/// Note: This updates the media online status.
pub fn update_inventory(
    drive: String,
    read_all_labels: Option<bool>,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {
    let upid_str = run_drive_worker(
        rpcenv,
        drive.clone(),
        "inventory-update",
        Some(drive.clone()),
        move |worker, config| {
            let (mut changer, changer_name) = required_media_changer(&config, &drive)?;

            let label_text_list = changer.online_media_label_texts()?;
            if label_text_list.is_empty() {
                worker.log("changer device does not list any media labels".to_string());
            }

            let state_path = Path::new(TAPE_STATUS_DIR);

            let mut inventory = Inventory::load(state_path)?;

            update_changer_online_status(&config, &mut inventory, &changer_name, &label_text_list)?;

            for label_text in label_text_list.iter() {
                if label_text.starts_with("CLN") {
                    worker.log(format!("skip cleaning unit '{}'", label_text));
                    continue;
                }

                let label_text = label_text.to_string();

                if !read_all_labels.unwrap_or(false) && inventory.find_media_by_label_text(&label_text).is_some() {
                    worker.log(format!("media '{}' already inventoried", label_text));
                    continue;
                }

                if let Err(err) = changer.load_media(&label_text) {
                    worker.warn(format!("unable to load media '{}' - {}", label_text, err));
                    continue;
                }

                let mut drive = open_drive(&config, &drive)?;
                match drive.read_label() {
                    Err(err) => {
                        worker.warn(format!("unable to read label form media '{}' - {}", label_text, err));
                    }
                    Ok((None, _)) => {
                        worker.log(format!("media '{}' is empty", label_text));
                    }
                    Ok((Some(media_id), _key_config)) => {
                        if label_text != media_id.label.label_text {
                            worker.warn(format!("label text mismatch ({} != {})", label_text, media_id.label.label_text));
                            continue;
                        }
                        worker.log(format!("inventorize media '{}' with uuid '{}'", label_text, media_id.label.uuid));

                        if let Some(MediaSetLabel { ref pool, ref uuid, ..}) =  media_id.media_set_label {
                            let _pool_lock = lock_media_pool(state_path, pool)?;
                            let _lock = lock_media_set(state_path, uuid, None)?;
                            MediaCatalog::destroy_unrelated_catalog(state_path, &media_id)?;
                            inventory.store(media_id, false)?;
                        } else {
                            let _lock = lock_unassigned_media_pool(state_path)?;
                            MediaCatalog::destroy(state_path, &media_id.label.uuid)?;
                            inventory.store(media_id, false)?;
                        };
                    }
                }
                changer.unload_media(None)?;
            }
            Ok(())
        },
    )?;

    Ok(upid_str.into())
}


#[api(
    input: {
        properties: {
            drive: {
                schema: DRIVE_NAME_SCHEMA,
            },
            pool: {
                schema: MEDIA_POOL_NAME_SCHEMA,
                optional: true,
            },
        },
    },
    returns: {
        schema: UPID_SCHEMA,
    },
    access: {
        permission: &Permission::Privilege(&["tape", "device", "{drive}"], PRIV_TAPE_WRITE, false),
    },
)]
/// Label media with barcodes from changer device
pub fn barcode_label_media(
    drive: String,
    pool: Option<String>,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {
    if let Some(ref pool) = pool {
        let (pool_config, _digest) = config::media_pool::config()?;

        if pool_config.sections.get(pool).is_none() {
            bail!("no such pool ('{}')", pool);
        }
    }

    let upid_str = run_drive_worker(
        rpcenv,
        drive.clone(),
        "barcode-label-media",
        Some(drive.clone()),
        move |worker, config| barcode_label_media_worker(worker, drive, &config, pool),
    )?;

    Ok(upid_str.into())
}

fn barcode_label_media_worker(
    worker: Arc<WorkerTask>,
    drive: String,
    drive_config: &SectionConfigData,
    pool: Option<String>,
) -> Result<(), Error> {
    let (mut changer, changer_name) = required_media_changer(drive_config, &drive)?;

    let mut label_text_list = changer.online_media_label_texts()?;

    // make sure we label them in the right order
    label_text_list.sort();

    let state_path = Path::new(TAPE_STATUS_DIR);

    let mut inventory = Inventory::load(state_path)?;

    update_changer_online_status(drive_config, &mut inventory, &changer_name, &label_text_list)?;

    if label_text_list.is_empty() {
        bail!("changer device does not list any media labels");
    }

    for label_text in label_text_list {
        if label_text.starts_with("CLN") { continue; }

        inventory.reload()?;
        if inventory.find_media_by_label_text(&label_text).is_some() {
            worker.log(format!("media '{}' already inventoried (already labeled)", label_text));
            continue;
        }

        worker.log(format!("checking/loading media '{}'", label_text));

        if let Err(err) = changer.load_media(&label_text) {
            worker.warn(format!("unable to load media '{}' - {}", label_text, err));
            continue;
        }

        let mut drive = open_drive(drive_config, &drive)?;
        drive.rewind()?;

        match drive.read_next_file() {
            Ok(Some(_file)) => {
                worker.log(format!("media '{}' is not empty (erase first)", label_text));
                continue;
            }
            Ok(None) => { /* EOF mark at BOT, assume tape is empty */ },
            Err(err) => {
                if err.is_errno(nix::errno::Errno::ENOSPC) || err.is_errno(nix::errno::Errno::EIO) {
                    /* assume tape is empty */
                } else {
                    worker.warn(format!("media '{}' read error (maybe not empty - erase first)", label_text));
                    continue;
                }
            }
        }

        let ctime = proxmox::tools::time::epoch_i64();
        let label = MediaLabel {
            label_text: label_text.to_string(),
            uuid: Uuid::generate(),
            ctime,
        };

        write_media_label(worker.clone(), &mut drive, label, pool.clone())?
    }

    Ok(())
}

#[api(
    input: {
        properties: {
            drive: {
                schema: DRIVE_NAME_SCHEMA,
            },
        },
    },
    returns: {
        description: "A List of medium auxiliary memory attributes.",
        type: Array,
        items: {
            type: MamAttribute,
        },
    },
    access: {
        permission: &Permission::Privilege(&["tape", "device", "{drive}"], PRIV_TAPE_AUDIT, false),
    },
)]
/// Read Cartridge Memory (Medium auxiliary memory attributes)
pub async fn cartridge_memory(drive: String) -> Result<Vec<MamAttribute>, Error> {
    run_drive_blocking_task(
        drive.clone(),
        "reading cartridge memory".to_string(),
        move |config| {
            let drive_config: LinuxTapeDrive = config.lookup("linux", &drive)?;
            let mut handle = drive_config.open()?;

            handle.cartridge_memory()
        }
    )
    .await
}

#[api(
    input: {
        properties: {
            drive: {
                schema: DRIVE_NAME_SCHEMA,
            },
        },
    },
    returns: {
        type: Lp17VolumeStatistics,
    },
    access: {
        permission: &Permission::Privilege(&["tape", "device", "{drive}"], PRIV_TAPE_AUDIT, false),
    },
)]
/// Read Volume Statistics (SCSI log page 17h)
pub async fn volume_statistics(drive: String) -> Result<Lp17VolumeStatistics, Error> {
    run_drive_blocking_task(
        drive.clone(),
        "reading volume statistics".to_string(),
        move |config| {
            let drive_config: LinuxTapeDrive = config.lookup("linux", &drive)?;
            let mut handle = drive_config.open()?;

            handle.volume_statistics()
        }
    )
    .await
}

#[api(
    input: {
        properties: {
            drive: {
                schema: DRIVE_NAME_SCHEMA,
            },
        },
    },
    returns: {
        type: LinuxDriveAndMediaStatus,
    },
    access: {
        permission: &Permission::Privilege(&["tape", "device", "{drive}"], PRIV_TAPE_AUDIT, false),
    },
)]
/// Get drive/media status
pub async fn status(drive: String) -> Result<LinuxDriveAndMediaStatus, Error> {
    run_drive_blocking_task(
        drive.clone(),
        "reading drive status".to_string(),
        move |config| {
            let drive_config: LinuxTapeDrive = config.lookup("linux", &drive)?;

            // Note: use open_linux_tape_device, because this also works if no medium loaded
            let file = open_linux_tape_device(&drive_config.path)?;

            let mut handle = LinuxTapeHandle::new(file);

            handle.get_drive_and_media_status()
        }
    )
    .await
}

#[api(
    input: {
        properties: {
            drive: {
                schema: DRIVE_NAME_SCHEMA,
            },
            force: {
                description: "Force overriding existing index.",
                type: bool,
                optional: true,
            },
            scan: {
                description: "Re-read the whole tape to reconstruct the catalog instead of restoring saved versions.",
                type: bool,
                optional: true,
            },
            verbose: {
                description: "Verbose mode - log all found chunks.",
                type: bool,
                optional: true,
            },
        },
    },
    returns: {
        schema: UPID_SCHEMA,
    },
    access: {
        permission: &Permission::Privilege(&["tape", "device", "{drive}"], PRIV_TAPE_READ, false),
    },
)]
/// Scan media and record content
pub fn catalog_media(
    drive: String,
    force: Option<bool>,
    scan: Option<bool>,
    verbose: Option<bool>,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {
    let verbose = verbose.unwrap_or(false);
    let force = force.unwrap_or(false);
    let scan = scan.unwrap_or(false);

    let upid_str = run_drive_worker(
        rpcenv,
        drive.clone(),
        "catalog-media",
        Some(drive.clone()),
        move |worker, config| {
            let mut drive = open_drive(&config, &drive)?;

            drive.rewind()?;

            let media_id = match drive.read_label()? {
                (Some(media_id), key_config) => {
                    worker.log(format!(
                        "found media label: {}",
                        serde_json::to_string_pretty(&serde_json::to_value(&media_id)?)?
                    ));
                    if key_config.is_some() {
                        worker.log(format!(
                            "encryption key config: {}",
                            serde_json::to_string_pretty(&serde_json::to_value(&key_config)?)?
                        ));
                    }
                    media_id
                },
                (None, _) => bail!("media is empty (no media label found)"),
            };

            let status_path = Path::new(TAPE_STATUS_DIR);

            let mut inventory = Inventory::new(status_path);

            let (_media_set_lock, media_set_uuid) = match media_id.media_set_label {
                None => {
                    worker.log("media is empty");
                    let _lock = lock_unassigned_media_pool(status_path)?;
                    MediaCatalog::destroy(status_path, &media_id.label.uuid)?;
                    inventory.store(media_id.clone(), false)?;
                    return Ok(());
                }
                Some(ref set) => {
                    if set.uuid.as_ref() == [0u8;16] { // media is empty
                        worker.log("media is empty");
                        let _lock = lock_unassigned_media_pool(status_path)?;
                        MediaCatalog::destroy(status_path, &media_id.label.uuid)?;
                        inventory.store(media_id.clone(), false)?;
                        return Ok(());
                    }
                    let encrypt_fingerprint = set.encryption_key_fingerprint.clone()
                        .map(|fp| (fp, set.uuid.clone()));

                    drive.set_encryption(encrypt_fingerprint)?;

                    let _pool_lock = lock_media_pool(status_path, &set.pool)?;
                    let media_set_lock = lock_media_set(status_path, &set.uuid, None)?;

                    MediaCatalog::destroy_unrelated_catalog(status_path, &media_id)?;

                    inventory.store(media_id.clone(), false)?;

                    (media_set_lock, &set.uuid)
                }
            };

            if MediaCatalog::exists(status_path, &media_id.label.uuid) && !force {
                bail!("media catalog exists (please use --force to overwrite)");
            }

            if !scan {
                let media_set = inventory.compute_media_set_members(media_set_uuid)?;

                if fast_catalog_restore(&worker, &mut drive, &media_set, &media_id.label.uuid)? {
                    return Ok(())
                }

                task_log!(worker, "no catalog found");
            }

            task_log!(worker, "scanning entire media to reconstruct catalog");

            drive.rewind()?;
            drive.read_label()?; // skip over labels - we already read them above

            restore_media(&worker, &mut drive, &media_id, None, verbose)?;

            Ok(())
        },
    )?;

    Ok(upid_str.into())
}

#[api(
    input: {
        properties: {
            changer: {
                schema: CHANGER_NAME_SCHEMA,
                optional: true,
            },
        },
    },
    returns: {
        description: "The list of configured drives with model information.",
        type: Array,
        items: {
            type: DriveListEntry,
        },
    },
    access: {
        description: "List configured tape drives filtered by Tape.Audit privileges",
        permission: &Permission::Anybody,
    },
)]
/// List drives
pub fn list_drives(
    changer: Option<String>,
    _param: Value,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<DriveListEntry>, Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let user_info = CachedUserInfo::new()?;

    let (config, _) = config::drive::config()?;

    let linux_drives = linux_tape_device_list();

    let drive_list: Vec<LinuxTapeDrive> = config.convert_to_typed_array("linux")?;

    let mut list = Vec::new();

    for drive in drive_list {
        if changer.is_some() && drive.changer != changer {
            continue;
        }

        let privs = user_info.lookup_privs(&auth_id, &["tape", "drive", &drive.name]);
        if (privs & PRIV_TAPE_AUDIT) == 0 {
            continue;
        }

        let info = lookup_device_identification(&linux_drives, &drive.path);
        let state = get_tape_device_state(&config, &drive.name)?;
        let entry = DriveListEntry { config: drive, info, state };
        list.push(entry);
    }

    Ok(list)
}

#[sortable]
pub const SUBDIRS: SubdirMap = &sorted!([
    (
        "barcode-label-media",
        &Router::new()
            .post(&API_METHOD_BARCODE_LABEL_MEDIA)
    ),
    (
        "catalog",
        &Router::new()
            .post(&API_METHOD_CATALOG_MEDIA)
    ),
    (
        "clean",
        &Router::new()
            .put(&API_METHOD_CLEAN_DRIVE)
    ),
    (
        "eject-media",
        &Router::new()
            .post(&API_METHOD_EJECT_MEDIA)
    ),
    (
        "erase-media",
        &Router::new()
            .post(&API_METHOD_ERASE_MEDIA)
    ),
    (
        "export-media",
        &Router::new()
            .put(&API_METHOD_EXPORT_MEDIA)
    ),
    (
        "inventory",
        &Router::new()
            .get(&API_METHOD_INVENTORY)
            .put(&API_METHOD_UPDATE_INVENTORY)
    ),
    (
        "label-media",
        &Router::new()
            .post(&API_METHOD_LABEL_MEDIA)
    ),
    (
        "load-media",
        &Router::new()
            .post(&API_METHOD_LOAD_MEDIA)
    ),
    (
        "load-slot",
        &Router::new()
            .put(&API_METHOD_LOAD_SLOT)
    ),
    (
        "cartridge-memory",
        &Router::new()
            .get(&API_METHOD_CARTRIDGE_MEMORY)
    ),
    (
        "volume-statistics",
        &Router::new()
            .get(&API_METHOD_VOLUME_STATISTICS)
    ),
    (
        "read-label",
        &Router::new()
            .get(&API_METHOD_READ_LABEL)
    ),
    (
        "restore-key",
        &Router::new()
            .post(&API_METHOD_RESTORE_KEY)
    ),
    (
        "rewind",
        &Router::new()
            .post(&API_METHOD_REWIND)
    ),
    (
        "status",
        &Router::new()
            .get(&API_METHOD_STATUS)
    ),
    (
        "unload",
        &Router::new()
            .post(&API_METHOD_UNLOAD)
    ),
]);

const ITEM_ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(SUBDIRS))
    .subdirs(&SUBDIRS);

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_LIST_DRIVES)
    .match_all("drive", &ITEM_ROUTER);
