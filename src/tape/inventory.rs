//! Backup media Inventory
//!
//! The Inventory persistently stores the list of known backup
//! media. A backup media is identified by its 'MediaId', which is the
//! MediaLabel/MediaSetLabel combination.
//!
//! Inventory Locking
//!
//! The inventory itself has several methods to update single entries,
//! but all of them can be considered atomic.
//!
//! Pool Locking
//!
//! To add/modify media assigned to a pool, we always do
//! lock_media_pool(). For unassigned media, we call
//! lock_unassigned_media_pool().
//!
//! MediaSet Locking
//!
//! To add/remove media from a media set, or to modify catalogs we
//! always do lock_media_set(). Also, we acquire this lock during
//! restore, to make sure it is not reused for backups.
//!

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Error};
use serde::{Deserialize, Serialize};
use serde_json::json;

use proxmox_sys::fs::{file_get_json, replace_file, CreateOptions};
use proxmox_uuid::Uuid;

use pbs_api_types::{MediaLocation, MediaSetPolicy, MediaStatus, RetentionPolicy};
use pbs_config::BackupLockGuard;

#[cfg(not(test))]
use pbs_config::open_backup_lockfile;

#[cfg(test)]
fn open_backup_lockfile<P: AsRef<std::path::Path>>(
    _path: P,
    _timeout: Option<std::time::Duration>,
    _exclusive: bool,
) -> Result<pbs_config::BackupLockGuard, anyhow::Error> {
    Ok(unsafe { pbs_config::create_mocked_lock() })
}

use crate::tape::{
    changer::OnlineStatusMap,
    file_formats::{MediaLabel, MediaSetLabel},
    MediaCatalog, MediaSet, TAPE_STATUS_DIR,
};

/// Unique Media Identifier
///
/// This combines the label and media set label.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MediaId {
    pub label: MediaLabel,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_set_label: Option<MediaSetLabel>,
}

impl MediaId {
    pub fn pool(&self) -> Option<String> {
        if let Some(set) = &self.media_set_label {
            return Some(set.pool.to_owned());
        }
        self.label.pool.to_owned()
    }
}

#[derive(Serialize, Deserialize)]
struct MediaStateEntry {
    id: MediaId,
    #[serde(skip_serializing_if = "Option::is_none")]
    location: Option<MediaLocation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<MediaStatus>,
}

/// Media Inventory
pub struct Inventory {
    map: BTreeMap<Uuid, MediaStateEntry>,

    inventory_path: PathBuf,
    lockfile_path: PathBuf,

    // helpers
    media_set_start_times: HashMap<Uuid, i64>,
}

impl Inventory {
    pub const MEDIA_INVENTORY_FILENAME: &'static str = "inventory.json";
    pub const MEDIA_INVENTORY_LOCKFILE: &'static str = ".inventory.lck";

    /// Create empty instance, no data loaded
    pub fn new<P: AsRef<Path>>(base_path: P) -> Self {
        let mut inventory_path = base_path.as_ref().to_owned();
        inventory_path.push(Self::MEDIA_INVENTORY_FILENAME);

        let mut lockfile_path = base_path.as_ref().to_owned();
        lockfile_path.push(Self::MEDIA_INVENTORY_LOCKFILE);

        Self {
            map: BTreeMap::new(),
            media_set_start_times: HashMap::new(),
            inventory_path,
            lockfile_path,
        }
    }

    pub fn load<P: AsRef<Path>>(base_path: P) -> Result<Self, Error> {
        let mut me = Self::new(base_path);
        me.reload()?;
        Ok(me)
    }

    /// Reload the database
    pub fn reload(&mut self) -> Result<(), Error> {
        self.map = self.load_media_db()?;
        self.update_helpers();
        Ok(())
    }

    fn update_helpers(&mut self) {
        // recompute media_set_start_times

        let mut set_start_times = HashMap::new();

        for entry in self.map.values() {
            let set = match &entry.id.media_set_label {
                None => continue,
                Some(set) => set,
            };
            if set.seq_nr == 0 {
                set_start_times.insert(set.uuid.clone(), set.ctime);
            }
        }

        self.media_set_start_times = set_start_times;
    }

    /// Lock the database
    fn lock(&self) -> Result<BackupLockGuard, Error> {
        open_backup_lockfile(&self.lockfile_path, None, true)
    }

    fn load_media_db(&self) -> Result<BTreeMap<Uuid, MediaStateEntry>, Error> {
        let data = file_get_json(&self.inventory_path, Some(json!([])))?;
        let media_list: Vec<MediaStateEntry> = serde_json::from_value(data)?;

        let mut map = BTreeMap::new();
        for entry in media_list.into_iter() {
            map.insert(entry.id.label.uuid.clone(), entry);
        }

        Ok(map)
    }

    fn replace_file(&self) -> Result<(), Error> {
        let list: Vec<&MediaStateEntry> = self.map.values().collect();
        let raw = serde_json::to_string_pretty(&serde_json::to_value(list)?)?;

        let mode = nix::sys::stat::Mode::from_bits_truncate(0o0640);

        let options = if cfg!(test) {
            // We cannot use chown inside test environment (no permissions)
            CreateOptions::new().perm(mode)
        } else {
            let backup_user = pbs_config::backup_user()?;
            CreateOptions::new()
                .perm(mode)
                .owner(backup_user.uid)
                .group(backup_user.gid)
        };

        replace_file(&self.inventory_path, raw.as_bytes(), options, true)?;

        Ok(())
    }

    /// Stores a single MediaID persistently
    pub fn store(&mut self, mut media_id: MediaId, clear_media_status: bool) -> Result<(), Error> {
        let _lock = self.lock()?;
        self.map = self.load_media_db()?;

        let uuid = media_id.label.uuid.clone();

        if let Some(previous) = self.map.remove(&media_id.label.uuid) {
            // do not overwrite unsaved pool assignments
            if media_id.media_set_label.is_none() {
                if let Some(ref set) = previous.id.media_set_label {
                    if set.unassigned() {
                        media_id.media_set_label = Some(set.clone());
                    }
                }
            }
            let entry = MediaStateEntry {
                id: media_id,
                location: previous.location,
                status: if clear_media_status {
                    None
                } else {
                    previous.status
                },
            };
            self.map.insert(uuid, entry);
        } else {
            let entry = MediaStateEntry {
                id: media_id,
                location: None,
                status: None,
            };
            self.map.insert(uuid, entry);
        }

        self.update_helpers();
        self.replace_file()?;
        Ok(())
    }

    /// Remove a single media persistently
    pub fn remove_media(&mut self, uuid: &Uuid) -> Result<(), Error> {
        let _lock = self.lock()?;
        self.map = self.load_media_db()?;
        self.map.remove(uuid);
        self.update_helpers();
        self.replace_file()?;
        Ok(())
    }

    /// Lookup media
    pub fn lookup_media(&self, uuid: &Uuid) -> Option<&MediaId> {
        self.map.get(uuid).map(|entry| &entry.id)
    }

    /// List all media Uuids
    pub fn media_list(&self) -> Vec<&Uuid> {
        self.map.keys().collect()
    }

    /// find media by label_text
    pub fn find_media_by_label_text(&self, label_text: &str) -> Result<Option<&MediaId>, Error> {
        let ids: Vec<_> = self
            .map
            .values()
            .filter_map(|entry| {
                if entry.id.label.label_text == label_text {
                    Some(&entry.id)
                } else {
                    None
                }
            })
            .collect();

        match ids.len() {
            0 => Ok(None),
            1 => Ok(Some(ids[0])),
            count => bail!("There are '{count}' tapes with the label '{label_text}'"),
        }
    }

    /// Lookup media pool
    ///
    /// Returns (pool, is_empty)
    pub fn lookup_media_pool(&self, uuid: &Uuid) -> Option<(&str, bool)> {
        let media_id = &self.map.get(uuid)?.id;
        match (&media_id.label.pool, &media_id.media_set_label) {
            (_, Some(media_set)) => Some((media_set.pool.as_str(), media_set.unassigned())),
            (Some(pool), None) => Some((pool.as_str(), true)),
            (None, None) => None,
        }
    }

    /// List all media assigned to the pool
    pub fn list_pool_media(&self, pool: &str) -> Vec<MediaId> {
        let mut list = Vec::new();

        for entry in self.map.values() {
            if entry.id.pool().as_deref() == Some(pool) {
                match entry.id.media_set_label {
                    Some(ref set) if set.unassigned() => list.push(MediaId {
                        label: entry.id.label.clone(),
                        media_set_label: None,
                    }),
                    _ => {
                        list.push(entry.id.clone());
                    }
                }
            }
        }

        list
    }

    /// List all used media
    pub fn list_used_media(&self) -> Vec<MediaId> {
        self.map
            .values()
            .filter_map(|entry| match entry.id.media_set_label {
                Some(ref set) if !set.unassigned() => Some(entry.id.clone()),
                _ => None,
            })
            .collect()
    }

    /// List media not assigned to any pool
    pub fn list_unassigned_media(&self) -> Vec<MediaId> {
        self.map
            .values()
            .filter_map(|entry| match entry.id.pool() {
                None => Some(entry.id.clone()),
                _ => None,
            })
            .collect()
    }

    pub fn media_set_start_time(&self, media_set_uuid: &Uuid) -> Option<i64> {
        self.media_set_start_times.get(media_set_uuid).copied()
    }

    /// Lookup media set pool
    pub fn lookup_media_set_pool(&self, media_set_uuid: &Uuid) -> Result<String, Error> {
        let mut last_pool = None;

        for entry in self.map.values() {
            match entry.id.media_set_label {
                None => continue,
                Some(MediaSetLabel { ref uuid, .. }) => {
                    if uuid != media_set_uuid {
                        continue;
                    }
                    if let Some((pool, _)) = self.lookup_media_pool(&entry.id.label.uuid) {
                        if let Some(last_pool) = last_pool {
                            if last_pool != pool {
                                bail!("detected media set with inconsistent pool assignment - internal error");
                            }
                        } else {
                            last_pool = Some(pool);
                        }
                    }
                }
            }
        }

        match last_pool {
            Some(pool) => Ok(pool.to_string()),
            None => bail!(
                "media set {} is incomplete - unable to lookup pool",
                media_set_uuid
            ),
        }
    }

    /// Compute a single media sets
    pub fn compute_media_set_members(&self, media_set_uuid: &Uuid) -> Result<MediaSet, Error> {
        let mut set = MediaSet::with_data(media_set_uuid.clone(), Vec::new());

        for entry in self.map.values() {
            match entry.id.media_set_label {
                None => continue,
                Some(MediaSetLabel {
                    seq_nr, ref uuid, ..
                }) => {
                    if uuid != media_set_uuid {
                        continue;
                    }
                    set.insert_media(entry.id.label.uuid.clone(), seq_nr)?;
                }
            }
        }

        Ok(set)
    }

    /// Compute all media sets
    pub fn compute_media_set_list(&self) -> Result<HashMap<Uuid, MediaSet>, Error> {
        let mut set_map: HashMap<Uuid, MediaSet> = HashMap::new();

        for entry in self.map.values() {
            match entry.id.media_set_label {
                None => continue,
                Some(MediaSetLabel {
                    seq_nr, ref uuid, ..
                }) => {
                    let set = set_map
                        .entry(uuid.clone())
                        .or_insert_with(|| MediaSet::with_data(uuid.clone(), Vec::new()));

                    set.insert_media(entry.id.label.uuid.clone(), seq_nr)?;
                }
            }
        }

        Ok(set_map)
    }

    /// Returns the latest media set for a pool
    pub fn latest_media_set(&self, pool: &str) -> Option<Uuid> {
        let mut last_set: Option<(Uuid, i64)> = None;

        let set_list = self
            .map
            .values()
            .filter_map(|entry| entry.id.media_set_label.as_ref())
            .filter(|set| set.pool == pool && !set.unassigned());

        for set in set_list {
            match last_set {
                None => {
                    last_set = Some((set.uuid.clone(), set.ctime));
                }
                Some((_, last_ctime)) => {
                    if set.ctime > last_ctime {
                        last_set = Some((set.uuid.clone(), set.ctime));
                    }
                }
            }
        }

        let (uuid, ctime) = match last_set {
            None => return None,
            Some((uuid, ctime)) => (uuid, ctime),
        };

        // consistency check - must be the only set with that ctime
        let set_list = self
            .map
            .values()
            .filter_map(|entry| entry.id.media_set_label.as_ref())
            .filter(|set| set.pool == pool && !set.unassigned());

        for set in set_list {
            if set.uuid != uuid && set.ctime >= ctime {
                // should not happen
                eprintln!(
                    "latest_media_set: found set with equal ctime ({}, {})",
                    set.uuid, uuid
                );
                return None;
            }
        }

        Some(uuid)
    }

    // Test if there is a media set (in the same pool) newer than this one.
    // Return the ctime of the nearest media set
    fn media_set_next_start_time(&self, media_set_uuid: &Uuid) -> Option<i64> {
        let (pool, ctime) = match self
            .map
            .values()
            .filter_map(|entry| entry.id.media_set_label.as_ref())
            .find_map(|set| {
                if &set.uuid == media_set_uuid {
                    Some((set.pool.clone(), set.ctime))
                } else {
                    None
                }
            }) {
            Some((pool, ctime)) => (pool, ctime),
            None => return None,
        };

        let set_list = self
            .map
            .values()
            .filter_map(|entry| entry.id.media_set_label.as_ref())
            .filter(|set| (&set.uuid != media_set_uuid) && (set.pool == pool));

        let mut next_ctime = None;

        for set in set_list {
            if set.ctime > ctime {
                match next_ctime {
                    None => {
                        next_ctime = Some(set.ctime);
                    }
                    Some(last_next_ctime) => {
                        if set.ctime < last_next_ctime {
                            next_ctime = Some(set.ctime);
                        }
                    }
                }
            }
        }

        next_ctime
    }

    pub fn media_expire_time(
        &self,
        media: &MediaId,
        media_set_policy: &MediaSetPolicy,
        retention_policy: &RetentionPolicy,
    ) -> i64 {
        if let RetentionPolicy::KeepForever = retention_policy {
            return i64::MAX;
        }

        let set = match media.media_set_label {
            None => return i64::MAX,
            Some(ref set) => set,
        };

        let set_start_time = match self.media_set_start_time(&set.uuid) {
            None => {
                // missing information, use ctime from this
                // set (always greater than ctime from seq_nr 0)
                set.ctime
            }
            Some(time) => time,
        };

        let max_use_time = match self.media_set_next_start_time(&set.uuid) {
            Some(next_start_time) => match media_set_policy {
                MediaSetPolicy::AlwaysCreate => set_start_time,
                _ => next_start_time,
            },
            None => match media_set_policy {
                MediaSetPolicy::ContinueCurrent => {
                    return i64::MAX;
                }
                MediaSetPolicy::AlwaysCreate => set_start_time,
                MediaSetPolicy::CreateAt(ref event) => {
                    match event.compute_next_event(set_start_time) {
                        Ok(Some(next)) => next,
                        Ok(None) | Err(_) => return i64::MAX,
                    }
                }
            },
        };

        match retention_policy {
            RetentionPolicy::KeepForever => i64::MAX,
            RetentionPolicy::OverwriteAlways => max_use_time,
            RetentionPolicy::ProtectFor(time_span) => {
                let seconds = f64::from(time_span.clone()) as i64;
                max_use_time + seconds
            }
        }
    }

    /// Generate a human readable name for the media set
    ///
    /// The template can include strftime time format specifications.
    pub fn generate_media_set_name(
        &self,
        media_set_uuid: &Uuid,
        template: Option<String>,
    ) -> Result<String, Error> {
        if let Some(ctime) = self.media_set_start_time(media_set_uuid) {
            let mut template = template.unwrap_or_else(|| String::from("%c"));
            template = template.replace("%id%", &media_set_uuid.to_string());
            Ok(proxmox_time::strftime_local(&template, ctime)?)
        } else {
            // We don't know the set start time, so we cannot use the template
            Ok(media_set_uuid.to_string())
        }
    }

    // Helpers to simplify testing

    /// Generate and insert a new free tape (test helper)
    pub fn generate_free_tape(&mut self, label_text: &str, ctime: i64) -> Uuid {
        let label = MediaLabel {
            label_text: label_text.to_string(),
            uuid: Uuid::generate(),
            ctime,
            pool: None,
        };
        let uuid = label.uuid.clone();

        self.store(
            MediaId {
                label,
                media_set_label: None,
            },
            false,
        )
        .unwrap();

        uuid
    }

    /// Generate and insert a new tape assigned to a specific pool
    /// (test helper)
    pub fn generate_assigned_tape(&mut self, label_text: &str, pool: &str, ctime: i64) -> Uuid {
        let label = MediaLabel {
            label_text: label_text.to_string(),
            uuid: Uuid::generate(),
            ctime,
            pool: Some(pool.to_string()),
        };

        let uuid = label.uuid.clone();

        self.store(
            MediaId {
                label,
                media_set_label: None,
            },
            false,
        )
        .unwrap();

        uuid
    }

    /// Generate and insert a used tape (test helper)
    pub fn generate_used_tape(&mut self, label_text: &str, set: MediaSetLabel, ctime: i64) -> Uuid {
        let label = MediaLabel {
            label_text: label_text.to_string(),
            uuid: Uuid::generate(),
            ctime,
            pool: Some(set.pool.clone()),
        };
        let uuid = label.uuid.clone();

        self.store(
            MediaId {
                label,
                media_set_label: Some(set),
            },
            false,
        )
        .unwrap();

        uuid
    }
}

// Status/location handling
impl Inventory {
    /// Returns status and location with reasonable defaults.
    ///
    /// Default status is 'MediaStatus::Unknown'.
    /// Default location is 'MediaLocation::Offline'.
    pub fn status_and_location(&self, uuid: &Uuid) -> (MediaStatus, MediaLocation) {
        match self.map.get(uuid) {
            None => {
                // no info stored - assume media is writable/offline
                (MediaStatus::Unknown, MediaLocation::Offline)
            }
            Some(entry) => {
                let location = entry.location.clone().unwrap_or(MediaLocation::Offline);
                let status = entry.status.unwrap_or(MediaStatus::Unknown);
                (status, location)
            }
        }
    }

    // Lock database, reload database, set status, store database
    fn set_media_status(&mut self, uuid: &Uuid, status: Option<MediaStatus>) -> Result<(), Error> {
        let _lock = self.lock()?;
        self.map = self.load_media_db()?;
        if let Some(entry) = self.map.get_mut(uuid) {
            entry.status = status;
            self.update_helpers();
            self.replace_file()?;
            Ok(())
        } else {
            bail!("no such media '{}'", uuid);
        }
    }

    /// Lock database, reload database, set status to Full, store database
    pub fn set_media_status_full(&mut self, uuid: &Uuid) -> Result<(), Error> {
        self.set_media_status(uuid, Some(MediaStatus::Full))
    }

    /// Lock database, reload database, set status to Damaged, store database
    pub fn set_media_status_damaged(&mut self, uuid: &Uuid) -> Result<(), Error> {
        self.set_media_status(uuid, Some(MediaStatus::Damaged))
    }

    /// Lock database, reload database, set status to Retired, store database
    pub fn set_media_status_retired(&mut self, uuid: &Uuid) -> Result<(), Error> {
        self.set_media_status(uuid, Some(MediaStatus::Retired))
    }

    /// Lock database, reload database, set status to None, store database
    pub fn clear_media_status(&mut self, uuid: &Uuid) -> Result<(), Error> {
        self.set_media_status(uuid, None)
    }

    // Lock database, reload database, set location, store database
    fn set_media_location(
        &mut self,
        uuid: &Uuid,
        location: Option<MediaLocation>,
    ) -> Result<(), Error> {
        let _lock = self.lock()?;
        self.map = self.load_media_db()?;
        if let Some(entry) = self.map.get_mut(uuid) {
            entry.location = location;
            self.update_helpers();
            self.replace_file()?;
            Ok(())
        } else {
            bail!("no such media '{}'", uuid);
        }
    }

    /// Lock database, reload database, set location to vault, store database
    pub fn set_media_location_vault(&mut self, uuid: &Uuid, vault: &str) -> Result<(), Error> {
        self.set_media_location(uuid, Some(MediaLocation::Vault(vault.to_string())))
    }

    /// Lock database, reload database, set location to offline, store database
    pub fn set_media_location_offline(&mut self, uuid: &Uuid) -> Result<(), Error> {
        self.set_media_location(uuid, Some(MediaLocation::Offline))
    }

    /// Update online status
    pub fn update_online_status(&mut self, online_map: &OnlineStatusMap) -> Result<(), Error> {
        let _lock = self.lock()?;
        self.map = self.load_media_db()?;

        for (uuid, entry) in self.map.iter_mut() {
            if let Some(changer_name) = online_map.lookup_changer(uuid) {
                entry.location = Some(MediaLocation::Online(changer_name.to_string()));
            } else if let Some(MediaLocation::Online(ref changer_name)) = entry.location {
                match online_map.online_map(changer_name) {
                    None => {
                        // no such changer device
                        entry.location = Some(MediaLocation::Offline);
                    }
                    Some(None) => {
                        // got no info - do nothing
                    }
                    Some(Some(_)) => {
                        // media changer changed
                        entry.location = Some(MediaLocation::Offline);
                    }
                }
            }
        }

        self.update_helpers();
        self.replace_file()?;

        Ok(())
    }
}

/// Lock a media pool
pub fn lock_media_pool<P: AsRef<Path>>(base_path: P, name: &str) -> Result<BackupLockGuard, Error> {
    let mut path = base_path.as_ref().to_owned();
    path.push(format!(".pool-{}", name));
    path.set_extension("lck");

    open_backup_lockfile(&path, None, true)
}

/// Lock for media not assigned to any pool
pub fn lock_unassigned_media_pool<P: AsRef<Path>>(base_path: P) -> Result<BackupLockGuard, Error> {
    // lock artificial "__UNASSIGNED__" pool to avoid races
    lock_media_pool(base_path, "__UNASSIGNED__")
}

/// Lock a media set
///
/// Timeout is 10 seconds by default
pub fn lock_media_set<P: AsRef<Path>>(
    base_path: P,
    media_set_uuid: &Uuid,
    timeout: Option<Duration>,
) -> Result<BackupLockGuard, Error> {
    let mut path = base_path.as_ref().to_owned();
    path.push(format!(".media-set-{}", media_set_uuid));
    path.set_extension("lck");

    open_backup_lockfile(&path, timeout, true)
}

// shell completion helper

/// List of known media uuids
pub fn complete_media_uuid(_arg: &str, _param: &HashMap<String, String>) -> Vec<String> {
    let inventory = match Inventory::load(TAPE_STATUS_DIR) {
        Ok(inventory) => inventory,
        Err(_) => return Vec::new(),
    };

    inventory.map.keys().map(|uuid| uuid.to_string()).collect()
}

/// List of known media sets
pub fn complete_media_set_uuid(_arg: &str, _param: &HashMap<String, String>) -> Vec<String> {
    let inventory = match Inventory::load(TAPE_STATUS_DIR) {
        Ok(inventory) => inventory,
        Err(_) => return Vec::new(),
    };

    inventory
        .map
        .values()
        .filter_map(|entry| entry.id.media_set_label.as_ref())
        .map(|set| set.uuid.to_string())
        .collect()
}

/// List of known media labels (barcodes)
pub fn complete_media_label_text(_arg: &str, _param: &HashMap<String, String>) -> Vec<String> {
    let inventory = match Inventory::load(TAPE_STATUS_DIR) {
        Ok(inventory) => inventory,
        Err(_) => return Vec::new(),
    };

    inventory
        .map
        .values()
        .map(|entry| entry.id.label.label_text.clone())
        .collect()
}

pub fn complete_media_set_snapshots(_arg: &str, param: &HashMap<String, String>) -> Vec<String> {
    let media_set_uuid: Uuid = match param.get("media-set").and_then(|s| s.parse().ok()) {
        Some(uuid) => uuid,
        None => return Vec::new(),
    };
    let inventory = match Inventory::load(TAPE_STATUS_DIR) {
        Ok(inventory) => inventory,
        Err(_) => return Vec::new(),
    };

    let mut res = Vec::new();
    let media_ids =
        inventory
            .list_used_media()
            .into_iter()
            .filter(|media| match &media.media_set_label {
                Some(label) => label.uuid == media_set_uuid,
                None => false,
            });

    for media_id in media_ids {
        let catalog = match MediaCatalog::open(TAPE_STATUS_DIR, &media_id, false, false) {
            Ok(catalog) => catalog,
            Err(_) => continue,
        };

        for (store, content) in catalog.content() {
            for snapshot in content.snapshot_index.keys() {
                res.push(format!("{}:{}", store, snapshot));
            }
        }
    }

    res
}
