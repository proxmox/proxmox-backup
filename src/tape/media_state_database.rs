use std::path::{Path, PathBuf};
use std::collections::BTreeMap;

use anyhow::Error;
use ::serde::{Deserialize, Serialize};
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
    tape::{
        OnlineStatusMap,
    },
    api2::types::{
        MediaStatus,
    },
};

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
/// Media location
pub enum MediaLocation {
    /// Ready for use (inside tape library)
    Online(String),
    /// Local available, but need to be mounted (insert into tape
    /// drive)
    Offline,
    /// Media is inside a Vault
    Vault(String),
}

#[derive(Serialize,Deserialize)]
struct MediaStateEntry {
    u: Uuid,
    #[serde(skip_serializing_if="Option::is_none")]
    l: Option<MediaLocation>,
    #[serde(skip_serializing_if="Option::is_none")]
    s: Option<MediaStatus>,
}

impl MediaStateEntry {
    fn new(uuid: Uuid) -> Self {
        MediaStateEntry { u: uuid, l: None, s: None }
    }
}

/// Stores MediaLocation and MediaState persistently
pub struct MediaStateDatabase {

    map: BTreeMap<Uuid, MediaStateEntry>,

    database_path: PathBuf,
    lockfile_path: PathBuf,
}

impl MediaStateDatabase {

    pub const MEDIA_STATUS_DATABASE_FILENAME: &'static str = "media-status-db.json";
    pub const MEDIA_STATUS_DATABASE_LOCKFILE: &'static str = ".media-status-db.lck";


    /// Lock the database
    pub fn lock(&self) -> Result<std::fs::File, Error> {
        open_file_locked(&self.lockfile_path, std::time::Duration::new(10, 0), true)
    }

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
                let location = entry.l.clone().unwrap_or(MediaLocation::Offline);
                let status = entry.s.unwrap_or(MediaStatus::Unknown);
                (status, location)
            }
        }
    }

    fn load_media_db(path: &Path) -> Result<BTreeMap<Uuid, MediaStateEntry>, Error> {

        let data = file_get_json(path, Some(json!([])))?;
        let list: Vec<MediaStateEntry> = serde_json::from_value(data)?;

        let mut map = BTreeMap::new();
        for entry in list.into_iter() {
            map.insert(entry.u.clone(), entry);
        }

        Ok(map)
    }

    /// Load the database into memory
    pub fn load(base_path: &Path) -> Result<MediaStateDatabase, Error> {

        let mut database_path = base_path.to_owned();
        database_path.push(Self::MEDIA_STATUS_DATABASE_FILENAME);

        let mut lockfile_path = base_path.to_owned();
        lockfile_path.push(Self::MEDIA_STATUS_DATABASE_LOCKFILE);

        Ok(MediaStateDatabase {
            map: Self::load_media_db(&database_path)?,
            database_path,
            lockfile_path,
        })
    }

    /// Lock database, reload database, set status to Full, store database
    pub fn set_media_status_full(&mut self, uuid: &Uuid) -> Result<(), Error> {
        let _lock = self.lock()?;
        self.map = Self::load_media_db(&self.database_path)?;
        let entry = self.map.entry(uuid.clone()).or_insert(MediaStateEntry::new(uuid.clone()));
        entry.s = Some(MediaStatus::Full);
        self.store()
    }

    /// Update online status
    pub fn update_online_status(&mut self, online_map: &OnlineStatusMap) -> Result<(), Error> {
        let _lock = self.lock()?;
        self.map = Self::load_media_db(&self.database_path)?;

        for (_uuid, entry) in self.map.iter_mut() {
            if let Some(changer_name) = online_map.lookup_changer(&entry.u) {
                entry.l = Some(MediaLocation::Online(changer_name.to_string()));
            } else {
                if let Some(MediaLocation::Online(ref changer_name)) = entry.l {
                    match online_map.online_map(changer_name) {
                        None => {
                            // no such changer device
                            entry.l = Some(MediaLocation::Offline);
                        }
                        Some(None) => {
                            // got no info - do nothing
                        }
                        Some(Some(_)) => {
                            // media changer changed
                            entry.l = Some(MediaLocation::Offline);
                        }
                    }
                }
            }
        }

        for (uuid, changer_name) in online_map.changer_map() {
            if self.map.contains_key(uuid) { continue; }
            let mut entry = MediaStateEntry::new(uuid.clone());
            entry.l = Some(MediaLocation::Online(changer_name.to_string()));
            self.map.insert(uuid.clone(), entry);
        }

        self.store()
    }

    /// Lock database, reload database, set status to Damaged, store database
    pub fn set_media_status_damaged(&mut self, uuid: &Uuid) -> Result<(), Error> {
        let _lock = self.lock()?;
        self.map = Self::load_media_db(&self.database_path)?;
        let entry = self.map.entry(uuid.clone()).or_insert(MediaStateEntry::new(uuid.clone()));
        entry.s = Some(MediaStatus::Damaged);
        self.store()
    }

    /// Lock database, reload database, set status to None, store database
    pub fn clear_media_status(&mut self, uuid: &Uuid) -> Result<(), Error> {
        let _lock = self.lock()?;
        self.map = Self::load_media_db(&self.database_path)?;
        let entry = self.map.entry(uuid.clone()).or_insert(MediaStateEntry::new(uuid.clone()));
        entry.s = None ;
        self.store()
    }

    /// Lock database, reload database, set location to vault, store database
    pub fn set_media_location_vault(&mut self, uuid: &Uuid, vault: &str) -> Result<(), Error> {
        let _lock = self.lock()?;
        self.map = Self::load_media_db(&self.database_path)?;
        let entry = self.map.entry(uuid.clone()).or_insert(MediaStateEntry::new(uuid.clone()));
        entry.l = Some(MediaLocation::Vault(vault.to_string()));
        self.store()
    }

    /// Lock database, reload database, set location to offline, store database
    pub fn set_media_location_offline(&mut self, uuid: &Uuid) -> Result<(), Error> {
        let _lock = self.lock()?;
        self.map = Self::load_media_db(&self.database_path)?;
        let entry = self.map.entry(uuid.clone()).or_insert(MediaStateEntry::new(uuid.clone()));
        entry.l = Some(MediaLocation::Offline);
        self.store()
    }

    /// Lock database, reload database, remove media, store database
    pub fn remove_media(&mut self, uuid: &Uuid) -> Result<(), Error> {
        let _lock = self.lock()?;
        self.map = Self::load_media_db(&self.database_path)?;
        self.map.remove(uuid);
        self.store()
    }

    fn store(&self) -> Result<(), Error> {

        let mut list = Vec::new();
        for entry in self.map.values() {
            list.push(entry);
        }

        let raw = serde_json::to_string_pretty(&serde_json::to_value(list)?)?;

        let backup_user = crate::backup::backup_user()?;
        let mode = nix::sys::stat::Mode::from_bits_truncate(0o0640);
        let options = CreateOptions::new()
            .perm(mode)
            .owner(backup_user.uid)
            .group(backup_user.gid);

        replace_file(&self.database_path, raw.as_bytes(), options)?;

        Ok(())
    }
}
