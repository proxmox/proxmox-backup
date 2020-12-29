use std::path::Path;

use anyhow::{bail, format_err, Error};
use serde::{Serialize, Deserialize};

use proxmox::{
    api::{api, Router, SubdirMap},
    list_subdirs_api_method,
};

use crate::{
    config::{
        self,
    },
    api2::types::{
        BACKUP_ID_SCHEMA,
        BACKUP_TYPE_SCHEMA,
        MEDIA_POOL_NAME_SCHEMA,
        MEDIA_LABEL_SCHEMA,
        MediaPoolConfig,
        MediaListEntry,
        MediaStatus,
        MediaContentEntry,
    },
    backup::{
        BackupDir,
    },
    tape::{
        TAPE_STATUS_DIR,
        Inventory,
        MediaStateDatabase,
        MediaPool,
        MediaCatalog,
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

#[api(
    properties: {
        pool: {
            schema: MEDIA_POOL_NAME_SCHEMA,
            optional: true,
        },
        "changer-id": {
            schema: MEDIA_LABEL_SCHEMA,
            optional: true,
        },
        "media": {
            description: "Filter by media UUID.",
            type: String,
            optional: true,
        },
        "media-set": {
            description: "Filter by media set UUID.",
            type: String,
            optional: true,
        },
        "backup-type": {
            schema: BACKUP_TYPE_SCHEMA,
            optional: true,
        },
        "backup-id": {
            schema: BACKUP_ID_SCHEMA,
            optional: true,
        },
    },
)]
#[derive(Serialize,Deserialize)]
#[serde(rename_all="kebab-case")]
/// Content list filter parameters
pub struct MediaContentListFilter {
    pub pool: Option<String>,
    pub changer_id: Option<String>,
    pub media: Option<String>,
    pub media_set: Option<String>,
    pub backup_type: Option<String>,
    pub backup_id: Option<String>,
}

#[api(
    input: {
        properties: {
            "filter": {
                type: MediaContentListFilter,
                flatten: true,
            },
        },
    },
    returns: {
        description: "Media content list.",
        type: Array,
        items: {
            type: MediaContentEntry,
        },
    },
)]
/// List media content
pub fn list_content(
    filter: MediaContentListFilter,
) -> Result<Vec<MediaContentEntry>, Error> {

    let (config, _digest) = config::media_pool::config()?;

    let status_path = Path::new(TAPE_STATUS_DIR);
    let inventory = Inventory::load(status_path)?;

    let media_uuid = filter.media.and_then(|s| s.parse().ok());
    let media_set_uuid = filter.media_set.and_then(|s| s.parse().ok());

    let mut list = Vec::new();

    for media_id in inventory.list_used_media() {
        let set = media_id.media_set_label.as_ref().unwrap();

        if let Some(ref changer_id) = filter.changer_id {
            if &media_id.label.changer_id != changer_id { continue; }
        }

        if let Some(ref pool) = filter.pool {
            if &set.pool != pool { continue; }
        }

        if let Some(ref media_uuid) = media_uuid {
            if &media_id.label.uuid != media_uuid { continue; }
        }

        if let Some(ref media_set_uuid) = media_set_uuid {
            if &set.uuid != media_set_uuid { continue; }
        }

        let config: MediaPoolConfig = config.lookup("pool", &set.pool)?;

        let media_set_name = inventory
            .generate_media_set_name(&set.uuid, config.template.clone())
            .unwrap_or_else(|_| set.uuid.to_string());

        let catalog = MediaCatalog::open(status_path, &media_id.label.uuid, false, false)?;

        for snapshot in catalog.snapshot_index().keys() {
            let backup_dir: BackupDir = snapshot.parse()?;

            if let Some(ref backup_type) = filter.backup_type {
                if backup_dir.group().backup_type() != backup_type { continue; }
            }
            if let Some(ref backup_id) = filter.backup_id {
                if backup_dir.group().backup_id() != backup_id { continue; }
            }

            list.push(MediaContentEntry {
                uuid: media_id.label.uuid.to_string(),
                changer_id: media_id.label.changer_id.to_string(),
                pool: set.pool.clone(),
                media_set_name: media_set_name.clone(),
                media_set_uuid: set.uuid.to_string(),
                seq_nr: set.seq_nr,
                snapshot: snapshot.to_owned(),
                backup_time: backup_dir.backup_time(),
            });
        }
    }

    Ok(list)
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
    (
        "content",
        &Router::new()
            .get(&API_METHOD_LIST_CONTENT)
    ),
];


pub const ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(SUBDIRS))
    .subdirs(SUBDIRS);
