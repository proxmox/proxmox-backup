use std::path::Path;

use anyhow::{bail, format_err, Error};

use proxmox::{
    api::{api, Router, SubdirMap},
    list_subdirs_api_method,
};

use crate::{
    config::{
        self,
    },
    api2::types::{
        MEDIA_POOL_NAME_SCHEMA,
        MEDIA_LABEL_SCHEMA,
        MediaPoolConfig,
        MediaListEntry,
        MediaStatus,
    },
    tape::{
        TAPE_STATUS_DIR,
        Inventory,
        MediaStateDatabase,
        MediaPool,
        update_online_status,
    },
};

#[api(
    input: {
        properties: {
            pool: {
                schema: MEDIA_POOL_NAME_SCHEMA,
                optional: true,
            },
        },
    },
    returns: {
        description: "List of registered backup media.",
        type: Array,
        items: {
            type: MediaListEntry,
        },
    },
)]
/// List pool media
pub async fn list_media(pool: Option<String>) -> Result<Vec<MediaListEntry>, Error> {

    let (config, _digest) = config::media_pool::config()?;

    let status_path = Path::new(TAPE_STATUS_DIR);

    tokio::task::spawn_blocking(move || {
        if let Err(err) = update_online_status(status_path) {
            eprintln!("{}", err);
            eprintln!("update online media status failed - using old state");
        }
    }).await?;

    let mut list = Vec::new();

    for (_section_type, data) in config.sections.values() {
        let pool_name = match data["name"].as_str() {
            None => continue,
            Some(name) => name,
        };
        if let Some(ref name) = pool {
            if name != pool_name {
                continue;
            }
        }

        let config: MediaPoolConfig = config.lookup("pool", pool_name)?;

        let pool = MediaPool::with_config(status_path, &config)?;

        let current_time = proxmox::tools::time::epoch_i64();

        for media in pool.list_media() {
            let expired = pool.media_is_expired(&media, current_time);

            let media_set_uuid = media.media_set_label().as_ref()
                .map(|set| set.uuid.to_string());

            let seq_nr = media.media_set_label().as_ref()
                .map(|set| set.seq_nr);

            let media_set_name = media.media_set_label().as_ref()
                .map(|set| {
                    pool.generate_media_set_name(&set.uuid, config.template.clone())
                        .unwrap_or_else(|_| set.uuid.to_string())
                });

            list.push(MediaListEntry {
                uuid: media.uuid().to_string(),
                changer_id: media.changer_id().to_string(),
                pool: Some(pool_name.to_string()),
                location: media.location().clone(),
                status: *media.status(),
                expired,
                media_set_uuid,
                media_set_name,
                seq_nr,
            });
        }
    }

    if pool.is_none() {

        let inventory = Inventory::load(status_path)?;
        let state_db = MediaStateDatabase::load(status_path)?;

        for media_id in inventory.list_unassigned_media() {

            let (mut status, location) = state_db.status_and_location(&media_id.label.uuid);

            if status == MediaStatus::Unknown {
                status = MediaStatus::Writable;
            }

            list.push(MediaListEntry {
                uuid: media_id.label.uuid.to_string(),
                changer_id: media_id.label.changer_id.to_string(),
                location,
                status,
                expired: false,
                media_set_uuid: None,
                media_set_name: None,
                seq_nr: None,
                pool: None,
            });
        }
    }

    Ok(list)
}

#[api(
    input: {
        properties: {
            "changer-id": {
                schema: MEDIA_LABEL_SCHEMA,
            },
            force: {
                description: "Force removal (even if media is used in a media set).",
                type: bool,
                optional: true,
            },
        },
    },
)]
/// Destroy media (completely remove from database)
pub fn destroy_media(changer_id: String, force: Option<bool>,) -> Result<(), Error> {

    let force = force.unwrap_or(false);

    let status_path = Path::new(TAPE_STATUS_DIR);
    let mut inventory = Inventory::load(status_path)?;

    let media_id = inventory.find_media_by_changer_id(&changer_id)
        .ok_or_else(|| format_err!("no such media '{}'", changer_id))?;

    if !force {
        if let Some(ref set) = media_id.media_set_label {
            let is_empty = set.uuid.as_ref() == [0u8;16];
            if !is_empty {
                bail!("media '{}' contains data (please use 'force' flag to remove.", changer_id);
            }
        }
    }

    let uuid = media_id.label.uuid.clone();
    drop(media_id);

    inventory.remove_media(&uuid)?;

    let mut state_db = MediaStateDatabase::load(status_path)?;
    state_db.remove_media(&uuid)?;

    Ok(())
}

const SUBDIRS: SubdirMap = &[
    (
        "destroy",
        &Router::new()
            .get(&API_METHOD_DESTROY_MEDIA)
    ),
    (
        "list",
        &Router::new()
            .get(&API_METHOD_LIST_MEDIA)
    ),
];


pub const ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(SUBDIRS))
    .subdirs(SUBDIRS);
