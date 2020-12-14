use std::path::Path;

use anyhow::Error;

use proxmox::api::{api, Router, SubdirMap};
use proxmox::list_subdirs_api_method;

use crate::{
    config::{
        self,
    },
    api2::types::{
        MEDIA_POOL_NAME_SCHEMA,
        MediaPoolConfig,
        MediaListEntry,
        MediaStatus,
        MediaLocationKind,
    },
    tape::{
        TAPE_STATUS_DIR,
        Inventory,
        MediaStateDatabase,
        MediaLocation,
        MediaPool,
        update_online_status,
    },
};

fn split_location(location: &MediaLocation) -> (MediaLocationKind, Option<String>) {
    match location {
        MediaLocation::Online(changer_name) => {
            (MediaLocationKind::Online, Some(changer_name.to_string()))
        }
        MediaLocation::Offline => {
            (MediaLocationKind::Offline, None)
        }
        MediaLocation::Vault(vault) => {
            (MediaLocationKind::Vault, Some(vault.to_string()))
        }
    }
}

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

        let pool = MediaPool::with_config(pool_name, status_path, &config)?;

        let current_time = proxmox::tools::time::epoch_i64();

        for media in pool.list_media() {
            let (location, location_hint) = split_location(&media.location());

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
                location,
                location_hint,
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
            let (location, location_hint) = split_location(&location);

            if status == MediaStatus::Unknown {
                status = MediaStatus::Writable;
            }

            list.push(MediaListEntry {
                uuid: media_id.label.uuid.to_string(),
                changer_id: media_id.label.changer_id.to_string(),
                location,
                location_hint,
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

const SUBDIRS: SubdirMap = &[
    (
        "list",
        &Router::new()
            .get(&API_METHOD_LIST_MEDIA)
    ),
];


pub const ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(SUBDIRS))
    .subdirs(SUBDIRS);
