use std::collections::HashMap;
use std::panic::UnwindSafe;
use std::sync::Arc;

use anyhow::{bail, format_err, Error};
use serde_json::Value;

use proxmox_router::{
    list_subdirs_api_method, Permission, Router, RpcEnvironment, RpcEnvironmentType, SubdirMap,
};
use proxmox_schema::api;
use proxmox_section_config::SectionConfigData;
use proxmox_sortable_macro::sortable;
use proxmox_sys::{task_log, task_warn};
use proxmox_uuid::Uuid;

use pbs_api_types::{
    Authid, DriveListEntry, LabelUuidMap, Lp17VolumeStatistics, LtoDriveAndMediaStatus,
    LtoTapeDrive, MamAttribute, MediaIdFlat, TapeDensity, CHANGER_NAME_SCHEMA, DRIVE_NAME_SCHEMA,
    MEDIA_LABEL_SCHEMA, MEDIA_POOL_NAME_SCHEMA, UPID_SCHEMA,
};

use pbs_api_types::{PRIV_TAPE_AUDIT, PRIV_TAPE_READ, PRIV_TAPE_WRITE};

use pbs_config::CachedUserInfo;
use pbs_tape::{
    linux_list_drives::{lookup_device_identification, lto_tape_device_list, open_lto_tape_device},
    sg_tape::tape_alert_flags_critical,
    BlockReadError,
};
use proxmox_rest_server::WorkerTask;

use crate::{
    api2::tape::restore::{fast_catalog_restore, restore_media},
    tape::{
        changer::update_changer_online_status,
        drive::{
            get_tape_device_state, lock_tape_device, media_changer, open_drive,
            open_lto_tape_drive, required_media_changer, set_tape_device_state, LtoTapeHandle,
            TapeDriver,
        },
        encryption_keys::insert_key,
        file_formats::{MediaLabel, MediaSetLabel},
        lock_media_pool, lock_media_set, lock_unassigned_media_pool, Inventory, MediaCatalog,
        MediaId, TAPE_STATUS_DIR,
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
    let (config, _digest) = pbs_config::drive::config()?;
    let lock_guard = lock_tape_device(&config, &drive)?;

    let auth_id = rpcenv.get_auth_id().unwrap();
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
    let (config, _digest) = pbs_config::drive::config()?;
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
            task_log!(
                worker,
                "loading media '{}' into drive '{}'",
                label_text,
                drive
            );
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
/// Format media. Check for label-text if given (cancels if wrong media).
pub fn format_media(
    drive: String,
    fast: Option<bool>,
    label_text: Option<String>,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {
    let upid_str = run_drive_worker(
        rpcenv,
        drive.clone(),
        "format-media",
        Some(drive.clone()),
        move |worker, config| {
            if let Some(ref label) = label_text {
                task_log!(worker, "try to load media '{}'", label);
                if let Some((mut changer, _)) = media_changer(&config, &drive)? {
                    changer.load_media(label)?;
                }
            }

            let mut handle = open_drive(&config, &drive)?;

            if !fast.unwrap_or(true) {
                let drive_config: LtoTapeDrive = config.lookup("lto", &drive)?;
                let file = open_lto_tape_device(&drive_config.path)?;
                let mut handle = LtoTapeHandle::new(file)?;
                if let Ok(status) = handle.get_drive_and_media_status() {
                    if status.density >= TapeDensity::LTO9 {
                        task_log!(worker, "Slow formatting LTO9+ media.");
                        task_log!(
                            worker,
                            "This can take a very long time due to media optimization."
                        );
                    }
                }
            }

            match handle.read_label() {
                Err(err) => {
                    if let Some(label) = label_text {
                        bail!("expected label '{}', found unrelated data", label);
                    }
                    /* assume drive contains no or unrelated data */
                    task_log!(worker, "unable to read media label: {}", err);
                    task_log!(worker, "format anyways");
                    handle.format_media(fast.unwrap_or(true))?;
                }
                Ok((None, _)) => {
                    if let Some(label) = label_text {
                        bail!("expected label '{}', found empty tape", label);
                    }
                    task_log!(worker, "found empty media - format anyways");
                    handle.format_media(fast.unwrap_or(true))?;
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
                        media_id.label.label_text,
                        media_id.label.uuid,
                    );

                    let mut inventory = Inventory::new(TAPE_STATUS_DIR);

                    let _pool_lock = if let Some(pool) = media_id.pool() {
                        lock_media_pool(TAPE_STATUS_DIR, &pool)?
                    } else {
                        lock_unassigned_media_pool(TAPE_STATUS_DIR)?
                    };

                    let _media_set_lock = match media_id.media_set_label {
                        Some(MediaSetLabel { ref uuid, .. }) => {
                            Some(lock_media_set(TAPE_STATUS_DIR, uuid, None)?)
                        }
                        None => None,
                    };

                    MediaCatalog::destroy(TAPE_STATUS_DIR, &media_id.label.uuid)?;
                    inventory.remove_media(&media_id.label.uuid)?;
                    drop(_media_set_lock);
                    drop(_pool_lock);

                    handle.format_media(fast.unwrap_or(true))?;
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
pub fn rewind(drive: String, rpcenv: &mut dyn RpcEnvironment) -> Result<Value, Error> {
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
pub fn eject_media(drive: String, rpcenv: &mut dyn RpcEnvironment) -> Result<Value, Error> {
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
/// Note: The media need to be empty (you may want to format it first).
pub fn label_media(
    drive: String,
    pool: Option<String>,
    label_text: String,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {
    if let Some(ref pool) = pool {
        let (pool_config, _digest) = pbs_config::media_pool::config()?;

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
                Ok(_reader) => bail!("media is not empty (format it first)"),
                Err(BlockReadError::EndOfFile) => { /* EOF mark at BOT, assume tape is empty */ }
                Err(BlockReadError::EndOfStream) => { /* tape is empty */ }
                Err(err) => {
                    bail!("media read error - {}", err);
                }
            }

            let ctime = proxmox_time::epoch_i64();
            let label = MediaLabel {
                label_text: label_text.to_string(),
                uuid: Uuid::generate(),
                ctime,
                pool: pool.clone(),
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
    let mut inventory = Inventory::new(TAPE_STATUS_DIR);
    inventory.reload()?;
    if inventory
        .find_media_by_label_text(&label.label_text)?
        .is_some()
    {
        bail!("Media with label '{}' already exists", label.label_text);
    }
    drive.label_tape(&label)?;
    if let Some(ref pool) = pool {
        task_log!(
            worker,
            "Label media '{}' for pool '{}'",
            label.label_text,
            pool
        );
    } else {
        task_log!(
            worker,
            "Label media '{}' (no pool assignment)",
            label.label_text
        );
    }

    let media_id = MediaId {
        label,
        media_set_label: None,
    };

    // Create the media catalog
    MediaCatalog::overwrite(TAPE_STATUS_DIR, &media_id, false)?;
    inventory.store(media_id.clone(), false)?;

    drive.rewind()?;

    match drive.read_label() {
        Ok((Some(info), _)) => {
            if info.label.uuid != media_id.label.uuid {
                bail!("verify label failed - got wrong label uuid");
            }
            if let Some(ref pool) = pool {
                match (info.label.pool, info.media_set_label) {
                    (None, Some(set)) => {
                        if !set.unassigned() {
                            bail!("verify media set label failed - got wrong set uuid");
                        }
                        if &set.pool != pool {
                            bail!("verify media set label failed - got wrong pool");
                        }
                    }
                    (Some(initial_pool), _) => {
                        if initial_pool != *pool {
                            bail!("verify media label failed - got wrong pool");
                        }
                    }
                    (None, None) => {
                        bail!("verify media set label failed (missing set label)");
                    }
                }
            }
        }
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
                //description: "Restore the key from this drive the (encrypted) key was saved on.",
            },
            password: {
                description: "The password the key was encrypted with.",
            },
        },
    },
    access: {
        permission: &Permission::Privilege(&["tape", "device", "{drive}"], PRIV_TAPE_READ, false),
    },
)]
/// Try to restore a tape encryption key
pub async fn restore_key(drive: String, password: String) -> Result<(), Error> {
    run_drive_blocking_task(drive.clone(), "restore key".to_string(), move |config| {
        let mut drive = open_drive(&config, &drive)?;

        let (_media_id, key_config) = drive.read_label()?;

        if let Some(key_config) = key_config {
            let password_fn = || Ok(password.as_bytes().to_vec());
            let (key, ..) = key_config.decrypt(&password_fn)?;
            insert_key(key, key_config, true)?;
        } else {
            bail!("media does not contain any encryption key configuration");
        }

        Ok(())
    })
    .await?;

    Ok(())
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
pub async fn read_label(drive: String, inventorize: Option<bool>) -> Result<MediaIdFlat, Error> {
    run_drive_blocking_task(drive.clone(), "reading label".to_string(), move |config| {
        let mut drive = open_drive(&config, &drive)?;

        let (media_id, _key_config) = drive.read_label()?;
        let media_id = media_id.ok_or_else(|| format_err!("Media is empty (no label)."))?;

        let label = if let Some(ref set) = media_id.media_set_label {
            let key = &set.encryption_key_fingerprint;

            if let Err(err) = drive.set_encryption(key.clone().map(|fp| (fp, set.uuid.clone()))) {
                eprintln!("unable to load encryption key: {}", err); // best-effort only
            }
            MediaIdFlat {
                ctime: media_id.label.ctime,
                encryption_key_fingerprint: key.as_ref().map(|fp| fp.signature()),
                label_text: media_id.label.label_text.clone(),
                media_set_ctime: Some(set.ctime),
                media_set_uuid: Some(set.uuid.clone()),
                pool: Some(set.pool.clone()),
                seq_nr: Some(set.seq_nr),
                uuid: media_id.label.uuid.clone(),
            }
        } else {
            MediaIdFlat {
                ctime: media_id.label.ctime,
                encryption_key_fingerprint: None,
                label_text: media_id.label.label_text.clone(),
                media_set_ctime: None,
                media_set_uuid: None,
                pool: media_id.label.pool.clone(),
                seq_nr: None,
                uuid: media_id.label.uuid.clone(),
            }
        };

        if let Some(true) = inventorize {
            let mut inventory = Inventory::new(TAPE_STATUS_DIR);

            let _pool_lock = if let Some(pool) = media_id.pool() {
                lock_media_pool(TAPE_STATUS_DIR, &pool)?
            } else {
                lock_unassigned_media_pool(TAPE_STATUS_DIR)?
            };

            if let Some(MediaSetLabel { ref uuid, .. }) = media_id.media_set_label {
                let _lock = lock_media_set(TAPE_STATUS_DIR, uuid, None)?;
                MediaCatalog::destroy_unrelated_catalog(TAPE_STATUS_DIR, &media_id)?;
            } else {
                MediaCatalog::destroy(TAPE_STATUS_DIR, &media_id.label.uuid)?;
            };

            inventory.store(media_id, false)?;
        }

        Ok(label)
    })
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
pub fn clean_drive(drive: String, rpcenv: &mut dyn RpcEnvironment) -> Result<Value, Error> {
    let upid_str = run_drive_worker(
        rpcenv,
        drive.clone(),
        "clean-drive",
        Some(drive.clone()),
        move |worker, config| {
            let (mut changer, _changer_name) = required_media_changer(&config, &drive)?;

            task_log!(worker, "Starting drive clean");

            changer.clean_drive()?;

            if let Ok(drive_config) = config.lookup::<LtoTapeDrive>("lto", &drive) {
                // Note: clean_drive unloads the cleaning media, so we cannot use drive_config.open
                let mut handle = LtoTapeHandle::new(open_lto_tape_device(&drive_config.path)?)?;

                // test for critical tape alert flags
                if let Ok(alert_flags) = handle.tape_alert_flags() {
                    if !alert_flags.is_empty() {
                        task_log!(worker, "TapeAlertFlags: {:?}", alert_flags);
                        if tape_alert_flags_critical(alert_flags) {
                            bail!("found critical tape alert flags: {:?}", alert_flags);
                        }
                    }
                }

                // test wearout (max. 50 mounts)
                if let Ok(volume_stats) = handle.volume_statistics() {
                    task_log!(worker, "Volume mounts: {}", volume_stats.volume_mounts);
                    let wearout = volume_stats.volume_mounts * 2; // (*100.0/50.0);
                    task_log!(worker, "Cleaning tape wearout: {}%", wearout);
                }
            }

            task_log!(worker, "Drive cleaned successfully");

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
pub async fn inventory(drive: String) -> Result<Vec<LabelUuidMap>, Error> {
    run_drive_blocking_task(drive.clone(), "inventorize".to_string(), move |config| {
        let (mut changer, changer_name) = required_media_changer(&config, &drive)?;

        let label_text_list = changer.online_media_label_texts()?;

        let mut inventory = Inventory::load(TAPE_STATUS_DIR)?;

        update_changer_online_status(&config, &mut inventory, &changer_name, &label_text_list)?;

        let mut list = Vec::new();

        for label_text in label_text_list.iter() {
            if label_text.starts_with("CLN") {
                // skip cleaning unit
                continue;
            }

            let label_text = label_text.to_string();

            match inventory.find_media_by_label_text(&label_text) {
                Ok(Some(media_id)) => {
                    list.push(LabelUuidMap {
                        label_text,
                        uuid: Some(media_id.label.uuid.clone()),
                    });
                }
                Ok(None) => {
                    list.push(LabelUuidMap {
                        label_text,
                        uuid: None,
                    });
                }
                Err(err) => {
                    log::warn!("error getting unique media label: {err}");
                    list.push(LabelUuidMap {
                        label_text,
                        uuid: None,
                    });
                }
            };
        }

        Ok(list)
    })
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
                default: false,
                optional: true,
            },
            "catalog": {
                description: "Restore the catalog from tape.",
                type: bool,
                default: false,
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
/// If `catalog` is true, also tries to restore the catalog from tape.
///
/// Note: This updates the media online status.
pub fn update_inventory(
    drive: String,
    read_all_labels: bool,
    catalog: bool,
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
                task_log!(worker, "changer device does not list any media labels");
            }

            let mut inventory = Inventory::load(TAPE_STATUS_DIR)?;

            update_changer_online_status(&config, &mut inventory, &changer_name, &label_text_list)?;

            for label_text in label_text_list.iter() {
                if label_text.starts_with("CLN") {
                    task_log!(worker, "skip cleaning unit '{}'", label_text);
                    continue;
                }

                let label_text = label_text.to_string();

                if !read_all_labels {
                    match inventory.find_media_by_label_text(&label_text) {
                        Ok(Some(media_id)) => {
                            if !catalog
                                || MediaCatalog::exists(TAPE_STATUS_DIR, &media_id.label.uuid)
                            {
                                task_log!(worker, "media '{}' already inventoried", label_text);
                                continue;
                            }
                        }
                        Err(err) => {
                            task_warn!(worker, "error getting media by unique label: {err}");
                            // we can't be sure which uuid it is
                            continue;
                        }
                        Ok(None) => {} // ok to inventorize
                    }
                }

                if let Err(err) = changer.load_media(&label_text) {
                    task_warn!(worker, "unable to load media '{}' - {}", label_text, err);
                    continue;
                }

                let mut drive = open_drive(&config, &drive)?;
                match drive.read_label() {
                    Err(err) => {
                        task_warn!(
                            worker,
                            "unable to read label form media '{}' - {}",
                            label_text,
                            err
                        );
                    }
                    Ok((None, _)) => {
                        task_log!(worker, "media '{}' is empty", label_text);
                    }
                    Ok((Some(media_id), _key_config)) => {
                        if label_text != media_id.label.label_text {
                            task_warn!(
                                worker,
                                "label text mismatch ({} != {})",
                                label_text,
                                media_id.label.label_text
                            );
                            continue;
                        }
                        task_log!(
                            worker,
                            "inventorize media '{}' with uuid '{}'",
                            label_text,
                            media_id.label.uuid
                        );

                        let _pool_lock = if let Some(pool) = media_id.pool() {
                            lock_media_pool(TAPE_STATUS_DIR, &pool)?
                        } else {
                            lock_unassigned_media_pool(TAPE_STATUS_DIR)?
                        };

                        if let Some(ref set) = media_id.media_set_label {
                            let _lock = lock_media_set(TAPE_STATUS_DIR, &set.uuid, None)?;
                            MediaCatalog::destroy_unrelated_catalog(TAPE_STATUS_DIR, &media_id)?;
                            inventory.store(media_id.clone(), false)?;

                            if set.unassigned() {
                                continue;
                            }

                            if catalog {
                                let media_set = inventory.compute_media_set_members(&set.uuid)?;
                                if let Err(err) = fast_catalog_restore(
                                    &worker,
                                    &mut drive,
                                    &media_set,
                                    &media_id.label.uuid,
                                ) {
                                    task_warn!(
                                        worker,
                                        "could not restore catalog for {label_text}: {err}"
                                    );
                                }
                            }
                        } else {
                            MediaCatalog::destroy(TAPE_STATUS_DIR, &media_id.label.uuid)?;
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
        let (pool_config, _digest) = pbs_config::media_pool::config()?;

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

    let mut inventory = Inventory::load(TAPE_STATUS_DIR)?;

    update_changer_online_status(
        drive_config,
        &mut inventory,
        &changer_name,
        &label_text_list,
    )?;

    if label_text_list.is_empty() {
        bail!("changer device does not list any media labels");
    }

    for label_text in label_text_list {
        if label_text.starts_with("CLN") {
            continue;
        }

        inventory.reload()?;
        match inventory.find_media_by_label_text(&label_text) {
            Ok(Some(_)) => {
                task_log!(
                    worker,
                    "media '{}' already inventoried (already labeled)",
                    label_text
                );
                continue;
            }
            Err(err) => {
                task_warn!(worker, "error getting media by unique label: {err}",);
                continue;
            }
            Ok(None) => {} // ok to label
        }

        task_log!(worker, "checking/loading media '{}'", label_text);

        if let Err(err) = changer.load_media(&label_text) {
            task_warn!(worker, "unable to load media '{}' - {}", label_text, err);
            continue;
        }

        let mut drive = open_drive(drive_config, &drive)?;
        drive.rewind()?;

        match drive.read_next_file() {
            Ok(_reader) => {
                task_log!(
                    worker,
                    "media '{}' is not empty (format it first)",
                    label_text
                );
                continue;
            }
            Err(BlockReadError::EndOfFile) => { /* EOF mark at BOT, assume tape is empty */ }
            Err(BlockReadError::EndOfStream) => { /* tape is empty */ }
            Err(_err) => {
                task_warn!(
                    worker,
                    "media '{}' read error (maybe not empty - format it first)",
                    label_text
                );
                continue;
            }
        }

        let ctime = proxmox_time::epoch_i64();
        let label = MediaLabel {
            label_text: label_text.to_string(),
            uuid: Uuid::generate(),
            ctime,
            pool: pool.clone(),
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
            let drive_config: LtoTapeDrive = config.lookup("lto", &drive)?;
            let mut handle = open_lto_tape_drive(&drive_config)?;

            handle.cartridge_memory()
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
            let drive_config: LtoTapeDrive = config.lookup("lto", &drive)?;
            let mut handle = open_lto_tape_drive(&drive_config)?;

            handle.volume_statistics()
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
        },
    },
    returns: {
        type: LtoDriveAndMediaStatus,
    },
    access: {
        permission: &Permission::Privilege(&["tape", "device", "{drive}"], PRIV_TAPE_AUDIT, false),
    },
)]
/// Get drive/media status
pub async fn status(drive: String) -> Result<LtoDriveAndMediaStatus, Error> {
    run_drive_blocking_task(
        drive.clone(),
        "reading drive status".to_string(),
        move |config| {
            let drive_config: LtoTapeDrive = config.lookup("lto", &drive)?;

            // Note: use open_lto_tape_device, because this also works if no medium loaded
            let file = open_lto_tape_device(&drive_config.path)?;

            let mut handle = LtoTapeHandle::new(file)?;

            handle.get_drive_and_media_status()
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
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;

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
                    task_log!(
                        worker,
                        "found media label: {}",
                        serde_json::to_string_pretty(&serde_json::to_value(&media_id)?)?
                    );
                    if key_config.is_some() {
                        task_log!(
                            worker,
                            "encryption key config: {}",
                            serde_json::to_string_pretty(&serde_json::to_value(&key_config)?)?
                        );
                    }
                    media_id
                }
                (None, _) => bail!("media is empty (no media label found)"),
            };

            let mut inventory = Inventory::new(TAPE_STATUS_DIR);

            let (_media_set_lock, media_set_uuid) = match media_id.media_set_label {
                None => {
                    task_log!(worker, "media is empty");
                    let _pool_lock = if let Some(pool) = media_id.pool() {
                        lock_media_pool(TAPE_STATUS_DIR, &pool)?
                    } else {
                        lock_unassigned_media_pool(TAPE_STATUS_DIR)?
                    };
                    MediaCatalog::destroy(TAPE_STATUS_DIR, &media_id.label.uuid)?;
                    inventory.store(media_id.clone(), false)?;
                    return Ok(());
                }
                Some(ref set) => {
                    if set.unassigned() {
                        // media is empty
                        task_log!(worker, "media is empty");
                        let _lock = lock_unassigned_media_pool(TAPE_STATUS_DIR)?;
                        MediaCatalog::destroy(TAPE_STATUS_DIR, &media_id.label.uuid)?;
                        inventory.store(media_id.clone(), false)?;
                        return Ok(());
                    }
                    let encrypt_fingerprint = set
                        .encryption_key_fingerprint
                        .clone()
                        .map(|fp| (fp, set.uuid.clone()));

                    drive.set_encryption(encrypt_fingerprint)?;

                    let _pool_lock = lock_media_pool(TAPE_STATUS_DIR, &set.pool)?;
                    let media_set_lock = lock_media_set(TAPE_STATUS_DIR, &set.uuid, None)?;

                    MediaCatalog::destroy_unrelated_catalog(TAPE_STATUS_DIR, &media_id)?;

                    inventory.store(media_id.clone(), false)?;

                    (media_set_lock, &set.uuid)
                }
            };

            if MediaCatalog::exists(TAPE_STATUS_DIR, &media_id.label.uuid) && !force {
                bail!("media catalog exists (please use --force to overwrite)");
            }

            if !scan {
                let media_set = inventory.compute_media_set_members(media_set_uuid)?;

                if fast_catalog_restore(&worker, &mut drive, &media_set, &media_id.label.uuid)? {
                    return Ok(());
                }

                task_log!(worker, "no catalog found");
            }

            task_log!(worker, "scanning entire media to reconstruct catalog");

            drive.rewind()?;
            drive.read_label()?; // skip over labels - we already read them above

            let mut checked_chunks = HashMap::new();
            restore_media(
                worker,
                &mut drive,
                &media_id,
                None,
                &mut checked_chunks,
                verbose,
                &auth_id,
            )?;

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

    let (config, _) = pbs_config::drive::config()?;

    let lto_drives = lto_tape_device_list();

    let drive_list: Vec<LtoTapeDrive> = config.convert_to_typed_array("lto")?;

    let mut list = Vec::new();

    for drive in drive_list {
        if changer.is_some() && drive.changer != changer {
            continue;
        }

        let privs = user_info.lookup_privs(&auth_id, &["tape", "drive", &drive.name]);
        if (privs & PRIV_TAPE_AUDIT) == 0 {
            continue;
        }

        let info = lookup_device_identification(&lto_drives, &drive.path);
        let state = get_tape_device_state(&config, &drive.name)?;
        let entry = DriveListEntry {
            config: drive,
            info,
            state,
        };
        list.push(entry);
    }

    Ok(list)
}

#[sortable]
pub const SUBDIRS: SubdirMap = &sorted!([
    (
        "barcode-label-media",
        &Router::new().post(&API_METHOD_BARCODE_LABEL_MEDIA)
    ),
    ("catalog", &Router::new().post(&API_METHOD_CATALOG_MEDIA)),
    ("clean", &Router::new().put(&API_METHOD_CLEAN_DRIVE)),
    ("eject-media", &Router::new().post(&API_METHOD_EJECT_MEDIA)),
    (
        "format-media",
        &Router::new().post(&API_METHOD_FORMAT_MEDIA)
    ),
    ("export-media", &Router::new().put(&API_METHOD_EXPORT_MEDIA)),
    (
        "inventory",
        &Router::new()
            .get(&API_METHOD_INVENTORY)
            .put(&API_METHOD_UPDATE_INVENTORY)
    ),
    ("label-media", &Router::new().post(&API_METHOD_LABEL_MEDIA)),
    ("load-media", &Router::new().post(&API_METHOD_LOAD_MEDIA)),
    ("load-slot", &Router::new().post(&API_METHOD_LOAD_SLOT)),
    (
        "cartridge-memory",
        &Router::new().get(&API_METHOD_CARTRIDGE_MEMORY)
    ),
    (
        "volume-statistics",
        &Router::new().get(&API_METHOD_VOLUME_STATISTICS)
    ),
    ("read-label", &Router::new().get(&API_METHOD_READ_LABEL)),
    ("restore-key", &Router::new().post(&API_METHOD_RESTORE_KEY)),
    ("rewind", &Router::new().post(&API_METHOD_REWIND)),
    ("status", &Router::new().get(&API_METHOD_STATUS)),
    ("unload", &Router::new().post(&API_METHOD_UNLOAD)),
]);

const ITEM_ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(SUBDIRS))
    .subdirs(SUBDIRS);

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_LIST_DRIVES)
    .match_all("drive", &ITEM_ROUTER);
