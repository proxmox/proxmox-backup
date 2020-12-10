use std::path::Path;
use anyhow::{bail, Error};
use serde_json::Value;

use proxmox::{
    sortable,
    identity,
    list_subdirs_api_method,
    tools::Uuid,
    sys::error::SysError,
    api::{
        api,
        Router,
        SubdirMap,
    },
};

use crate::{
    config,
    api2::types::{
        DRIVE_ID_SCHEMA,
        MEDIA_LABEL_SCHEMA,
        MEDIA_POOL_NAME_SCHEMA,
        LinuxTapeDrive,
        ScsiTapeChanger,
        TapeDeviceInfo,
    },
    tape::{
        TAPE_STATUS_DIR,
        TapeDriver,
        MediaChange,
        Inventory,
        MediaId,
        mtx_load,
        mtx_unload,
        linux_tape_device_list,
        open_drive,
        media_changer,
        file_formats::{
            DriveLabel,
            MediaSetLabel,
        },
    },
};

#[api(
    input: {
        properties: {
            drive: {
                schema: DRIVE_ID_SCHEMA,
            },
            slot: {
                description: "Source slot number",
                minimum: 1,
            },
        },
    },
)]
/// Load media via changer from slot
pub fn load_slot(
    drive: String,
    slot: u64,
    _param: Value,
) -> Result<(), Error> {

    let (config, _digest) = config::drive::config()?;

    let drive_config: LinuxTapeDrive = config.lookup("linux", &drive)?;

    let changer: ScsiTapeChanger = match drive_config.changer {
        Some(ref changer) => config.lookup("changer", changer)?,
        None => bail!("drive '{}' has no associated changer", drive),
    };

    let drivenum = drive_config.changer_drive_id.unwrap_or(0);

    mtx_load(&changer.path, slot, drivenum)
}

#[api(
    input: {
        properties: {
            drive: {
                schema: DRIVE_ID_SCHEMA,
            },
            "changer-id": {
                schema: MEDIA_LABEL_SCHEMA,
            },
        },
    },
)]
/// Load media with specified label
///
/// Issue a media load request to the associated changer device.
pub fn load_media(drive: String, changer_id: String) -> Result<(), Error> {

    let (config, _digest) = config::drive::config()?;

    let (mut changer, _) = media_changer(&config, &drive, false)?;

    changer.load_media(&changer_id)?;

    Ok(())
}

#[api(
    input: {
        properties: {
            drive: {
                schema: DRIVE_ID_SCHEMA,
            },
            slot: {
                description: "Target slot number. If omitted, defaults to the slot that the drive was loaded from.",
                minimum: 1,
                optional: true,
            },
        },
    },
)]
/// Unload media via changer
pub fn unload(
    drive: String,
    slot: Option<u64>,
    _param: Value,
) -> Result<(), Error> {

    let (config, _digest) = config::drive::config()?;

    let mut drive_config: LinuxTapeDrive = config.lookup("linux", &drive)?;

    let changer: ScsiTapeChanger = match drive_config.changer {
        Some(ref changer) => config.lookup("changer", changer)?,
        None => bail!("drive '{}' has no associated changer", drive),
    };

    let drivenum: u64 = 0;

    if let Some(slot) = slot {
        mtx_unload(&changer.path, slot, drivenum)
    } else {
        drive_config.unload_media()
    }
}

#[api(
    input: {
        properties: {},
    },
    returns: {
        description: "The list of autodetected tape drives.",
        type: Array,
        items: {
            type: TapeDeviceInfo,
        },
    },
)]
/// Scan tape drives
pub fn scan_drives(_param: Value) -> Result<Vec<TapeDeviceInfo>, Error> {

    let list = linux_tape_device_list();

    Ok(list)
}

#[api(
    input: {
        properties: {
            drive: {
                schema: DRIVE_ID_SCHEMA,
            },
            fast: {
                description: "Use fast erase.",
                type: bool,
                optional: true,
                default: true,
            },
        },
    },
)]
/// Erase media
pub fn erase_media(drive: String, fast: Option<bool>) -> Result<(), Error> {

    let (config, _digest) = config::drive::config()?;

    let mut drive = open_drive(&config, &drive)?;

    drive.erase_media(fast.unwrap_or(true))?;

    Ok(())
}

#[api(
    input: {
        properties: {
            drive: {
                schema: DRIVE_ID_SCHEMA,
            },
        },
    },
)]
/// Rewind tape
pub fn rewind(drive: String) -> Result<(), Error> {

    let (config, _digest) = config::drive::config()?;

    let mut drive = open_drive(&config, &drive)?;

    drive.rewind()?;

    Ok(())
}

#[api(
    input: {
        properties: {
            drive: {
                schema: DRIVE_ID_SCHEMA,
            },
        },
    },
)]
/// Eject/Unload drive media
pub fn eject_media(drive: String) -> Result<(), Error> {

    let (config, _digest) = config::drive::config()?;

    let (mut changer, _) = media_changer(&config, &drive, false)?;

    if !changer.eject_on_unload() {
        let mut drive = open_drive(&config, &drive)?;
        drive.eject_media()?;
    }

    changer.unload_media()?;

    Ok(())
}

#[api(
    input: {
        properties: {
            drive: {
                schema: DRIVE_ID_SCHEMA,
            },
            "changer-id": {
                schema: MEDIA_LABEL_SCHEMA,
            },
            pool: {
                schema: MEDIA_POOL_NAME_SCHEMA,
                optional: true,
            },
        },
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
    changer_id: String,
) -> Result<(), Error> {

    if let Some(ref pool) = pool {
        let (pool_config, _digest) = config::media_pool::config()?;

        if pool_config.sections.get(pool).is_none() {
            bail!("no such pool ('{}')", pool);
        }
    }

    let (config, _digest) = config::drive::config()?;

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
    let label = DriveLabel {
        changer_id: changer_id.to_string(),
        uuid: Uuid::generate(),
        ctime,
    };

    write_media_label(&mut drive, label, pool)
}

fn write_media_label(
    drive: &mut Box<dyn TapeDriver>,
    label: DriveLabel,
    pool: Option<String>,
) -> Result<(), Error> {

    drive.label_tape(&label)?;

    let mut media_set_label = None;

    if let Some(ref pool) = pool {
        // assign media to pool by writing special media set label
        println!("Label media '{}' for pool '{}'", label.changer_id, pool);
        let set = MediaSetLabel::with_data(&pool, [0u8; 16].into(), 0, label.ctime);

        drive.write_media_set_label(&set)?;
        media_set_label = Some(set);
    } else {
        println!("Label media '{}' (no pool assignment)", label.changer_id);
    }

    let media_id = MediaId { label, media_set_label };

    let mut inventory = Inventory::load(Path::new(TAPE_STATUS_DIR))?;
    inventory.store(media_id.clone())?;

    drive.rewind()?;

    match drive.read_label() {
        Ok(Some(info)) => {
            if info.label.uuid != media_id.label.uuid {
                bail!("verify label failed - got wrong label uuid");
            }
            if let Some(ref pool) = pool {
                match info.media_set_label {
                    Some((set, _)) => {
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
        Ok(None) => bail!("verify label failed (got empty media)"),
        Err(err) => bail!("verify label failed - {}", err),
    };

    drive.rewind()?;

    Ok(())
}

#[sortable]
pub const SUBDIRS: SubdirMap = &sorted!([
    (
        "eject-media",
        &Router::new()
            .put(&API_METHOD_EJECT_MEDIA)
    ),
    (
        "erase-media",
        &Router::new()
            .put(&API_METHOD_ERASE_MEDIA)
    ),
    (
        "label-media",
        &Router::new()
            .put(&API_METHOD_LABEL_MEDIA)
    ),
    (
        "load-slot",
        &Router::new()
            .put(&API_METHOD_LOAD_SLOT)
    ),
    (
        "rewind",
        &Router::new()
            .put(&API_METHOD_REWIND)
    ),
    (
        "scan",
        &Router::new()
            .get(&API_METHOD_SCAN_DRIVES)
    ),
    (
        "unload",
        &Router::new()
            .put(&API_METHOD_UNLOAD)
    ),
]);

pub const ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(SUBDIRS))
    .subdirs(SUBDIRS);
