//! Backup media Inventory
//!
//! The Inventory persistently stores the list of known backup
//! media. A backup media is identified by its 'MediaId', which is the
//! DriveLabel/MediaSetLabel combination.

use std::collections::{HashMap, BTreeMap};
use std::path::{Path, PathBuf};

use anyhow::{bail, Error};
use serde::{Serialize, Deserialize};
use serde_json::json;

use proxmox::tools::{
    Uuid,
    fs::{
        open_file_locked,
        replace_file,
        file_get_json,
        CreateOptions,
    },
};

use crate::{
    tools::systemd::time::compute_next_event,
    api2::types::{
        MediaSetPolicy,
        RetentionPolicy,
    },
    tape::{
        MEDIA_POOL_STATUS_DIR,
        file_formats::{
            DriveLabel,
            MediaSetLabel,
        },
    },
};

/// Unique Media Identifier
///
/// This combines the label and media set label.
#[derive(Debug,Serialize,Deserialize,Clone)]
pub struct MediaId {
    pub label: DriveLabel,
    #[serde(skip_serializing_if="Option::is_none")]
    pub media_set_label: Option<MediaSetLabel>,
}

/// Media Set
///
/// A List of backup media
#[derive(Debug, Serialize, Deserialize)]
pub struct MediaSet {
    /// Unique media set ID
    uuid: Uuid,
    /// List of BackupMedia
    media_list: Vec<Option<Uuid>>,
}

impl MediaSet {

    pub const MEDIA_SET_MAX_SEQ_NR: u64 = 100;

    pub fn new() -> Self {
        let uuid = Uuid::generate();
        Self {
            uuid,
            media_list: Vec::new(),
        }
    }

    pub fn with_data(uuid: Uuid, media_list: Vec<Option<Uuid>>) -> Self {
        Self { uuid, media_list }
    }

    pub fn uuid(&self) -> &Uuid {
        &self.uuid
    }

    pub fn media_list(&self) -> &[Option<Uuid>] {
        &self.media_list
    }

    pub fn add_media(&mut self, uuid: Uuid) {
        self.media_list.push(Some(uuid));
    }

    pub fn insert_media(&mut self, uuid: Uuid, seq_nr: u64) -> Result<(), Error> {
        if seq_nr > Self::MEDIA_SET_MAX_SEQ_NR {
            bail!("media set sequence number to large in media set {} ({} > {})",
                  self.uuid.to_string(), seq_nr, Self::MEDIA_SET_MAX_SEQ_NR);
        }
        let seq_nr = seq_nr as usize;
        if self.media_list.len() > seq_nr {
            if self.media_list[seq_nr].is_some() {
                bail!("found duplicate squence number in media set '{}/{}'",
                      self.uuid.to_string(), seq_nr);
            }
        } else {
            self.media_list.resize(seq_nr + 1, None);
        }
        self.media_list[seq_nr] = Some(uuid);
        Ok(())
    }

    pub fn last_media_uuid(&self) -> Option<&Uuid> {
        match self.media_list.last() {
            None => None,
            Some(None) => None,
            Some(Some(ref last_uuid)) => Some(last_uuid),
        }
    }

    pub fn is_last_media(&self, uuid: &Uuid) -> bool {
        match self.media_list.last() {
            None => false,
            Some(None) => false,
            Some(Some(last_uuid)) => uuid == last_uuid,
        }
    }
}

/// Media Inventory
pub struct Inventory {
    map: BTreeMap<Uuid, MediaId>,

    inventory_path: PathBuf,
    lockfile_path: PathBuf,

    // helpers
    media_set_start_times: HashMap<Uuid, i64>
}

impl Inventory {

    pub const MEDIA_INVENTORY_FILENAME: &'static str = "inventory.json";
    pub const MEDIA_INVENTORY_LOCKFILE: &'static str = ".inventory.lck";

    fn new(base_path: &Path) -> Self {

        let mut inventory_path = base_path.to_owned();
        inventory_path.push(Self::MEDIA_INVENTORY_FILENAME);

        let mut lockfile_path = base_path.to_owned();
        lockfile_path.push(Self::MEDIA_INVENTORY_LOCKFILE);

        Self {
            map: BTreeMap::new(),
            media_set_start_times: HashMap::new(),
            inventory_path,
            lockfile_path,
        }
    }

    pub fn load(base_path: &Path) -> Result<Self, Error> {
        let mut me = Self::new(base_path);
        me.reload()?;
        Ok(me)
    }

    /// Reload the database
    pub fn reload(&mut self) -> Result<(), Error> {
        self.map = Self::load_media_db(&self.inventory_path)?;
        self.update_helpers();
        Ok(())
    }

    fn update_helpers(&mut self) {

        // recompute media_set_start_times

        let mut set_start_times = HashMap::new();

        for media in self.map.values() {
            let set = match &media.media_set_label {
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
    pub fn lock(&self) -> Result<std::fs::File, Error> {
        open_file_locked(&self.lockfile_path, std::time::Duration::new(10, 0), true)
    }

    fn load_media_db(path: &Path) -> Result<BTreeMap<Uuid, MediaId>, Error> {

        let data = file_get_json(path, Some(json!([])))?;
        let media_list: Vec<MediaId> = serde_json::from_value(data)?;

        let mut map = BTreeMap::new();
        for item in media_list.into_iter() {
            map.insert(item.label.uuid.clone(), item);
        }

        Ok(map)
    }

    fn replace_file(&self) -> Result<(), Error> {
        let list: Vec<&MediaId> = self.map.values().collect();
        let raw = serde_json::to_string_pretty(&serde_json::to_value(list)?)?;
        let options = CreateOptions::new();
        replace_file(&self.inventory_path, raw.as_bytes(), options)?;
        Ok(())
    }

    /// Stores a single MediaID persistently
    pub fn store(&mut self, mut media_id: MediaId) -> Result<(), Error> {
        let _lock = self.lock()?;
        self.map = Self::load_media_db(&self.inventory_path)?;

        // do not overwrite unsaved pool assignments
        if media_id.media_set_label.is_none() {
            if let Some(previous) = self.map.get(&media_id.label.uuid) {
                if let Some(ref set) = previous.media_set_label {
                    if set.uuid.as_ref() == [0u8;16] {
                        media_id.media_set_label = Some(set.clone());
                    }
                }
            }
        }

        self.map.insert(media_id.label.uuid.clone(), media_id);
        self.update_helpers();
        self.replace_file()?;
        Ok(())
    }

    /*
    /// Same a store, but extract MediaId form MediaLabelInfo
    pub fn store_media_info(&mut self, info: &MediaLabelInfo) -> Result<(), Error> {
        let media_id = MediaId {
            label: info.label.clone(),
            media_set_label: info.media_set_label.clone().map(|(l, _)| l),
        };
        self.store(media_id)
    }
    */

    /// Lookup media
    pub fn lookup_media(&self, uuid: &Uuid) -> Option<&MediaId> {
        self.map.get(uuid)
    }

    /// find media by changer_id
    pub fn find_media_by_changer_id(&self, changer_id: &str) -> Option<&MediaId> {
        for (_uuid, media_id) in &self.map {
            if media_id.label.changer_id == changer_id {
                return Some(media_id);
            }
        }
        None
    }

    /// Lookup media pool
    ///
    /// Returns (pool, is_empty)
    pub fn lookup_media_pool(&self, uuid: &Uuid) -> Option<(&str, bool)> {
        match self.map.get(uuid) {
            None => None,
            Some(media_id) => {
                match media_id.media_set_label {
                    None => None, // not assigned to any pool
                    Some(ref set) => {
                        let is_empty = set.uuid.as_ref() == [0u8;16];
                        Some((&set.pool, is_empty))
                    }
                }
            }
        }
    }

    /// List all media assigned to the pool
    pub fn list_pool_media(&self, pool: &str) -> Vec<MediaId> {
        let mut list = Vec::new();

        for (_uuid, media_id) in &self.map {
            match media_id.media_set_label {
                None => continue, // not assigned to any pool
                Some(ref set) => {
                    if set.pool != pool {
                        continue; // belong to another pool
                    }

                    if set.uuid.as_ref() == [0u8;16] { // should we do this??
                        list.push(MediaId {
                            label: media_id.label.clone(),
                            media_set_label: None,
                        })
                    } else {
                        list.push(media_id.clone());
                    }
                }
            }

        }

        list
    }

    /// List all used media
    pub fn list_used_media(&self) -> Vec<MediaId> {
        let mut list = Vec::new();

        for (_uuid, media_id) in &self.map {
            match media_id.media_set_label {
                None => continue, // not assigned to any pool
                Some(ref set) => {
                    if set.uuid.as_ref() != [0u8;16] {
                        list.push(media_id.clone());
                    }
                }
            }
        }

        list
    }

    /// List media not assigned to any pool
    pub fn list_unassigned_media(&self) -> Vec<MediaId> {
        let mut list = Vec::new();

        for (_uuid, media_id) in &self.map {
            if media_id.media_set_label.is_none() {
                list.push(media_id.clone());
            }
        }

        list
    }

    pub fn media_set_start_time(&self, media_set_uuid: &Uuid) -> Option<i64> {
        self.media_set_start_times.get(media_set_uuid).map(|t| *t)
    }

    /// Compute a single media sets
    pub fn compute_media_set_members(&self, media_set_uuid: &Uuid) -> Result<MediaSet, Error> {

        let mut set = MediaSet::with_data(media_set_uuid.clone(), Vec::new());

        for media in self.map.values() {
            match media.media_set_label {
                None => continue,
                Some(MediaSetLabel { seq_nr, ref uuid, .. }) => {
                    if  uuid != media_set_uuid {
                        continue;
                    }
                    set.insert_media(media.label.uuid.clone(), seq_nr)?;
                }
            }
        }

        Ok(set)
    }

    /// Compute all media sets
    pub fn compute_media_set_list(&self) -> Result<HashMap<Uuid, MediaSet>, Error> {

        let mut set_map: HashMap<Uuid, MediaSet> = HashMap::new();

        for media in self.map.values() {
            match media.media_set_label {
                None => continue,
                Some(MediaSetLabel { seq_nr, ref uuid, .. }) => {

                    let set = set_map.entry(uuid.clone()).or_insert_with(|| {
                        MediaSet::with_data(uuid.clone(), Vec::new())
                    });

                    set.insert_media(media.label.uuid.clone(), seq_nr)?;
                }
            }
        }

        Ok(set_map)
    }

    /// Returns the latest media set for a pool
    pub fn latest_media_set(&self, pool: &str) -> Option<Uuid> {

        let mut last_set: Option<(Uuid, i64)> = None;

        let set_list = self.map.values()
            .filter_map(|media| media.media_set_label.as_ref())
            .filter(|set| &set.pool == &pool && set.uuid.as_ref() != [0u8;16]);

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
        let set_list = self.map.values()
            .filter_map(|media| media.media_set_label.as_ref())
            .filter(|set| &set.pool == &pool && set.uuid.as_ref() != [0u8;16]);

        for set in set_list {
            if set.uuid != uuid && set.ctime >= ctime { // should not happen
                eprintln!("latest_media_set: found set with equal ctime ({}, {})", set.uuid, uuid);
                return None;
            }
        }

        Some(uuid)
    }

    // Test if there is a media set (in the same pool) newer than this one.
    // Return the ctime of the nearest media set
    fn media_set_next_start_time(&self, media_set_uuid: &Uuid) -> Option<i64> {

        let (pool, ctime) = match self.map.values()
            .filter_map(|media| media.media_set_label.as_ref())
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

        let set_list = self.map.values()
            .filter_map(|media| media.media_set_label.as_ref())
            .filter(|set| (&set.uuid != media_set_uuid) && (&set.pool == &pool));

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

        let max_use_time = match media_set_policy {
            MediaSetPolicy::ContinueCurrent => {
                match self.media_set_next_start_time(&set.uuid) {
                    Some(next_start_time) => next_start_time,
                    None => return i64::MAX,
                }
            }
            MediaSetPolicy::AlwaysCreate => {
                set_start_time + 1
            }
            MediaSetPolicy::CreateAt(ref event) => {
                match compute_next_event(event, set_start_time, false) {
                    Ok(Some(next)) => next,
                    Ok(None) | Err(_) => return i64::MAX,
                }
            }
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
            let mut template = template.unwrap_or(String::from("%id%"));
            template = template.replace("%id%", &media_set_uuid.to_string());
            proxmox::tools::time::strftime_local(&template, ctime)
        } else {
            // We don't know the set start time, so we cannot use the template
            Ok(media_set_uuid.to_string())
        }
    }

    // Helpers to simplify testing

    /// Genreate and insert a new free tape (test helper)
    pub fn generate_free_tape(&mut self, changer_id: &str, ctime: i64) -> Uuid {

        let label = DriveLabel {
            changer_id: changer_id.to_string(),
            uuid: Uuid::generate(),
            ctime,
        };
        let uuid = label.uuid.clone();

        self.store(MediaId { label, media_set_label: None }).unwrap();

        uuid
    }

    /// Genreate and insert a new tape assigned to a specific pool
    /// (test helper)
    pub fn generate_assigned_tape(
        &mut self,
        changer_id: &str,
        pool: &str,
        ctime: i64,
    ) -> Uuid {

        let label = DriveLabel {
            changer_id: changer_id.to_string(),
            uuid: Uuid::generate(),
            ctime,
        };

        let uuid = label.uuid.clone();

        let set = MediaSetLabel::with_data(pool, [0u8; 16].into(), 0, ctime);

        self.store(MediaId { label, media_set_label: Some(set) }).unwrap();

        uuid
    }

    /// Genreate and insert a used tape (test helper)
    pub fn generate_used_tape(
        &mut self,
        changer_id: &str,
        set: MediaSetLabel,
        ctime: i64,
    ) -> Uuid {
        let label = DriveLabel {
            changer_id: changer_id.to_string(),
            uuid: Uuid::generate(),
            ctime,
        };
        let uuid = label.uuid.clone();

        self.store(MediaId { label, media_set_label: Some(set) }).unwrap();

        uuid
    }
}

// shell completion helper

/// List of known media uuids
pub fn complete_media_uuid(
    _arg: &str,
    _param: &HashMap<String, String>,
) -> Vec<String> {

    let inventory = match Inventory::load(Path::new(MEDIA_POOL_STATUS_DIR)) {
        Ok(inventory) => inventory,
        Err(_) => return Vec::new(),
    };

    inventory.map.keys().map(|uuid| uuid.to_string()).collect()
}

/// List of known media sets
pub fn complete_media_set_uuid(
    _arg: &str,
    _param: &HashMap<String, String>,
) -> Vec<String> {

    let inventory = match Inventory::load(Path::new(MEDIA_POOL_STATUS_DIR)) {
        Ok(inventory) => inventory,
        Err(_) => return Vec::new(),
    };

    inventory.map.values()
        .filter_map(|media| media.media_set_label.as_ref())
        .map(|set| set.uuid.to_string()).collect()
}

/// List of known media labels (barcodes)
pub fn complete_media_changer_id(
    _arg: &str,
    _param: &HashMap<String, String>,
) -> Vec<String> {

    let inventory = match Inventory::load(Path::new(MEDIA_POOL_STATUS_DIR)) {
        Ok(inventory) => inventory,
        Err(_) => return Vec::new(),
    };

    inventory.map.values().map(|media| media.label.changer_id.clone()).collect()
}
