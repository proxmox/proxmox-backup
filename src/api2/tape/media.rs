use std::path::Path;

use anyhow::{bail, format_err, Error};
use serde::{Serialize, Deserialize};

use proxmox::{
    api::{api, Router, SubdirMap},
    list_subdirs_api_method,
    tools::Uuid,
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
        MEDIA_UUID_SCHEMA,
        MEDIA_SET_UUID_SCHEMA,
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

    let catalogs = tokio::task::spawn_blocking(move || {
        // update online media status
        if let Err(err) = update_online_status(status_path) {
            eprintln!("{}", err);
            eprintln!("update online media status failed - using old state");
        }
        // test what catalog files we have
        MediaCatalog::media_with_catalogs(status_path)
    }).await??;

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

        let use_offline_media = true; // does not matter here
        let pool = MediaPool::with_config(status_path, &config, use_offline_media)?;

        let current_time = proxmox::tools::time::epoch_i64();

        for media in pool.list_media() {
            let expired = pool.media_is_expired(&media, current_time);

            let media_set_uuid = media.media_set_label()
                .map(|set| set.uuid.clone());

            let seq_nr = media.media_set_label()
                .map(|set| set.seq_nr);

            let media_set_name = media.media_set_label()
                .map(|set| {
                    pool.generate_media_set_name(&set.uuid, config.template.clone())
                        .unwrap_or_else(|_| set.uuid.to_string())
                });

            let catalog_ok = if media.media_set_label().is_none() {
                // Media is empty, we need no catalog
                true
            } else {
                catalogs.contains(media.uuid())
            };

            list.push(MediaListEntry {
                uuid: media.uuid().clone(),
                label_text: media.label_text().to_string(),
                ctime: media.ctime(),
                pool: Some(pool_name.to_string()),
                location: media.location().clone(),
                status: *media.status(),
                catalog: catalog_ok,
                expired,
                media_set_ctime: media.media_set_label().map(|set| set.ctime),
                media_set_uuid,
                media_set_name,
                seq_nr,
            });
        }
    }

    if pool.is_none() {

        let inventory = Inventory::load(status_path)?;

        for media_id in inventory.list_unassigned_media() {

            let (mut status, location) = inventory.status_and_location(&media_id.label.uuid);

            if status == MediaStatus::Unknown {
                status = MediaStatus::Writable;
            }

            list.push(MediaListEntry {
                uuid: media_id.label.uuid.clone(),
                ctime: media_id.label.ctime,
                label_text: media_id.label.label_text.to_string(),
                location,
                status,
                catalog: true, // empty, so we do not need a catalog
                expired: false,
                media_set_uuid: None,
                media_set_name: None,
                media_set_ctime: None,
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
            "label-text": {
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
pub fn destroy_media(label_text: String, force: Option<bool>,) -> Result<(), Error> {

    let force = force.unwrap_or(false);

    let status_path = Path::new(TAPE_STATUS_DIR);
    let mut inventory = Inventory::load(status_path)?;

    let media_id = inventory.find_media_by_label_text(&label_text)
        .ok_or_else(|| format_err!("no such media '{}'", label_text))?;

    if !force {
        if let Some(ref set) = media_id.media_set_label {
            let is_empty = set.uuid.as_ref() == [0u8;16];
            if !is_empty {
                bail!("media '{}' contains data (please use 'force' flag to remove.", label_text);
            }
        }
    }

    let uuid = media_id.label.uuid.clone();
    drop(media_id);

    inventory.remove_media(&uuid)?;

    Ok(())
}

#[api(
    properties: {
        pool: {
            schema: MEDIA_POOL_NAME_SCHEMA,
            optional: true,
        },
        "label-text": {
            schema: MEDIA_LABEL_SCHEMA,
            optional: true,
        },
        "media": {
            schema: MEDIA_UUID_SCHEMA,
            optional: true,
        },
        "media-set": {
            schema: MEDIA_SET_UUID_SCHEMA,
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
    pub label_text: Option<String>,
    pub media: Option<Uuid>,
    pub media_set: Option<Uuid>,
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

    let mut list = Vec::new();

    for media_id in inventory.list_used_media() {
        let set = media_id.media_set_label.as_ref().unwrap();

        if let Some(ref label_text) = filter.label_text {
            if &media_id.label.label_text != label_text { continue; }
        }

        if let Some(ref pool) = filter.pool {
            if &set.pool != pool { continue; }
        }

        if let Some(ref media_uuid) = filter.media {
            if &media_id.label.uuid != media_uuid { continue; }
        }

        if let Some(ref media_set_uuid) = filter.media_set {
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
                uuid: media_id.label.uuid.clone(),
                label_text: media_id.label.label_text.to_string(),
                pool: set.pool.clone(),
                media_set_name: media_set_name.clone(),
                media_set_uuid: set.uuid.clone(),
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
        "content",
        &Router::new()
            .get(&API_METHOD_LIST_CONTENT)
    ),
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
