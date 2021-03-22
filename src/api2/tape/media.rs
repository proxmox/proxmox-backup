use std::path::Path;

use anyhow::{bail, format_err, Error};
use serde::{Serialize, Deserialize};

use proxmox::{
    api::{api, Router, SubdirMap, RpcEnvironment, Permission},
    list_subdirs_api_method,
    tools::Uuid,
};

use crate::{
    config::{
        self,
        cached_user_info::CachedUserInfo,
        acl::{
            PRIV_TAPE_AUDIT,
        },
    },
    api2::types::{
        Authid,
        BACKUP_ID_SCHEMA,
        BACKUP_TYPE_SCHEMA,
        MEDIA_POOL_NAME_SCHEMA,
        MEDIA_LABEL_SCHEMA,
        MEDIA_UUID_SCHEMA,
        MEDIA_SET_UUID_SCHEMA,
        CHANGER_NAME_SCHEMA,
        MediaPoolConfig,
        MediaListEntry,
        MediaStatus,
        MediaContentEntry,
        VAULT_NAME_SCHEMA,
    },
    backup::{
        BackupDir,
    },
    tape::{
        TAPE_STATUS_DIR,
        Inventory,
        MediaPool,
        MediaCatalog,
        changer::update_online_status,
    },
};

#[api(
    input: {
        properties: {
            pool: {
                schema: MEDIA_POOL_NAME_SCHEMA,
                optional: true,
            },
            "update-status": {
                description: "Try to update tape library status (check what tapes are online).",
                optional: true,
                default: true,
            },
            "update-status-changer": {
                // only update status for a single changer
                schema: CHANGER_NAME_SCHEMA,
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
    access: {
        description: "List of registered backup media filtered by Tape.Audit privileges on pool",
        permission: &Permission::Anybody,
    },
)]
/// List pool media
pub async fn list_media(
    pool: Option<String>,
    update_status: bool,
    update_status_changer: Option<String>,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<MediaListEntry>, Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let user_info = CachedUserInfo::new()?;

    let (config, _digest) = config::media_pool::config()?;

    let status_path = Path::new(TAPE_STATUS_DIR);

    let catalogs = tokio::task::spawn_blocking(move || {
        if update_status {
            // update online media status
            if let Err(err) = update_online_status(status_path, update_status_changer.as_deref()) {
                eprintln!("{}", err);
                eprintln!("update online media status failed - using old state");
            }
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

        let privs = user_info.lookup_privs(&auth_id, &["tape", "pool", pool_name]);
        if (privs & PRIV_TAPE_AUDIT) == 0  {
            continue;
        }

        let config: MediaPoolConfig = config.lookup("pool", pool_name)?;

        let changer_name = None; // assume standalone drive
        let mut pool = MediaPool::with_config(status_path, &config, changer_name, true)?;

        let current_time = proxmox::tools::time::epoch_i64();

        // Call start_write_session, so that we show the same status a
        // backup job would see.
        pool.force_media_availability();
        pool.start_write_session(current_time)?;

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

    let inventory = Inventory::load(status_path)?;

    let privs = user_info.lookup_privs(&auth_id, &["tape", "pool"]);
    if (privs & PRIV_TAPE_AUDIT) != 0  {
        if pool.is_none() {

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
    }

    // add media with missing pool configuration
    // set status to MediaStatus::Unknown
    for uuid in inventory.media_list() {
        let media_id = inventory.lookup_media(uuid).unwrap();
        let media_set_label = match media_id.media_set_label {
            Some(ref set) => set,
            None => continue,
        };

        if config.sections.get(&media_set_label.pool).is_some() {
            continue;
        }

        let privs = user_info.lookup_privs(&auth_id, &["tape", "pool", &media_set_label.pool]);
        if (privs & PRIV_TAPE_AUDIT) == 0  {
            continue;
        }

        let (_status, location) = inventory.status_and_location(uuid);

        let media_set_name = inventory.generate_media_set_name(&media_set_label.uuid, None)?;

        list.push(MediaListEntry {
            uuid: media_id.label.uuid.clone(),
            label_text: media_id.label.label_text.clone(),
            ctime: media_id.label.ctime,
            pool: Some(media_set_label.pool.clone()),
            location,
            status: MediaStatus::Unknown,
            catalog: catalogs.contains(uuid),
            expired: false,
            media_set_ctime: Some(media_set_label.ctime),
            media_set_uuid: Some(media_set_label.uuid.clone()),
            media_set_name: Some(media_set_name),
            seq_nr: Some(media_set_label.seq_nr),
        });

    }


    Ok(list)
}

#[api(
    input: {
        properties: {
            "label-text": {
                schema: MEDIA_LABEL_SCHEMA,
            },
            "vault-name": {
                schema: VAULT_NAME_SCHEMA,
                optional: true,
            },
        },
    },
)]
/// Change Tape location to vault (if given), or offline.
pub fn move_tape(
    label_text: String,
    vault_name: Option<String>,
) -> Result<(), Error> {

    let status_path = Path::new(TAPE_STATUS_DIR);
    let mut inventory = Inventory::load(status_path)?;

    let uuid = inventory.find_media_by_label_text(&label_text)
        .ok_or_else(|| format_err!("no such media '{}'", label_text))?
        .label
        .uuid
        .clone();

    if let Some(vault_name) = vault_name {
        inventory.set_media_location_vault(&uuid, &vault_name)?;
    } else {
        inventory.set_media_location_offline(&uuid)?;
    }

    Ok(())
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
    access: {
        description: "List content filtered by Tape.Audit privilege on pool",
        permission: &Permission::Anybody,
    },
)]
/// List media content
pub fn list_content(
    filter: MediaContentListFilter,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<MediaContentEntry>, Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let user_info = CachedUserInfo::new()?;

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

        let privs = user_info.lookup_privs(&auth_id, &["tape", "pool", &set.pool]);
        if (privs & PRIV_TAPE_AUDIT) == 0  {
            continue;
        }

        if let Some(ref media_uuid) = filter.media {
            if &media_id.label.uuid != media_uuid { continue; }
        }

        if let Some(ref media_set_uuid) = filter.media_set {
            if &set.uuid != media_set_uuid { continue; }
        }

        let template = match config.lookup::<MediaPoolConfig>("pool", &set.pool) {
            Ok(pool_config) => pool_config.template.clone(),
            _ => None, // simply use default if there is no pool config
        };

        let media_set_name = inventory
            .generate_media_set_name(&set.uuid, template)
            .unwrap_or_else(|_| set.uuid.to_string());

        let catalog = MediaCatalog::open(status_path, &media_id, false, false)?;

        for (store, content) in catalog.content() {
            for snapshot in content.snapshot_index.keys() {
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
                    media_set_ctime: set.ctime,
                    seq_nr: set.seq_nr,
                    snapshot: snapshot.to_owned(),
                    store: store.to_owned(),
                    backup_time: backup_dir.backup_time(),
                });
            }
        }
    }

    Ok(list)
}

#[api(
    input: {
        properties: {
            uuid: {
                schema: MEDIA_UUID_SCHEMA,
            },
        },
    },
)]
/// Get current media status
pub fn get_media_status(uuid: Uuid) -> Result<MediaStatus, Error> {

    let status_path = Path::new(TAPE_STATUS_DIR);
    let inventory = Inventory::load(status_path)?;

    let (status, _location) = inventory.status_and_location(&uuid);

    Ok(status)
}

#[api(
    input: {
        properties: {
            uuid: {
                schema: MEDIA_UUID_SCHEMA,
            },
            status: {
                type: MediaStatus,
                optional: true,
            },
        },
    },
)]
/// Update media status (None, 'full', 'damaged' or 'retired')
///
/// It is not allowed to set status to 'writable' or 'unknown' (those
/// are internally managed states).
pub fn update_media_status(uuid: Uuid, status: Option<MediaStatus>) -> Result<(), Error> {

    let status_path = Path::new(TAPE_STATUS_DIR);
    let mut inventory = Inventory::load(status_path)?;

    match status {
        None => inventory.clear_media_status(&uuid)?,
        Some(MediaStatus::Retired) => inventory.set_media_status_retired(&uuid)?,
        Some(MediaStatus::Damaged) => inventory.set_media_status_damaged(&uuid)?,
        Some(MediaStatus::Full) => inventory.set_media_status_full(&uuid)?,
        Some(status) => bail!("setting media status '{:?}' is not allowed", status),
    }

    Ok(())
}

const MEDIA_SUBDIRS: SubdirMap = &[
    (
        "status",
        &Router::new()
            .get(&API_METHOD_GET_MEDIA_STATUS)
            .post(&API_METHOD_UPDATE_MEDIA_STATUS)
    ),
];

pub const MEDIA_ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(MEDIA_SUBDIRS))
    .subdirs(MEDIA_SUBDIRS);

pub const MEDIA_LIST_ROUTER: Router = Router::new()
    .get(&API_METHOD_LIST_MEDIA)
    .match_all("uuid", &MEDIA_ROUTER);

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
    ( "list", &MEDIA_LIST_ROUTER ),
    (
        "move",
        &Router::new()
            .post(&API_METHOD_MOVE_TAPE)
    ),
];


pub const ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(SUBDIRS))
    .subdirs(SUBDIRS);
