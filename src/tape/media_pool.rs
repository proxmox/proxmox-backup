//! Media Pool
//!
//! A set of backup medias.
//!
//! This struct manages backup media state during backup. The main
//! purpose is to allocate media sets and assing new tapes to it.
//!
//!

use std::path::Path;
use anyhow::{bail, Error};
use ::serde::{Deserialize, Serialize};

use proxmox::tools::Uuid;

use crate::{
    api2::types::{
        MediaStatus,
        MediaLocation,
        MediaSetPolicy,
        RetentionPolicy,
        MediaPoolConfig,
    },
    tools::systemd::time::compute_next_event,
    tape::{
        MediaId,
        MediaSet,
        Inventory,
        MediaStateDatabase,
        file_formats::{
            MediaLabel,
            MediaSetLabel,
        },
    }
};

/// Media Pool lock guard
pub struct MediaPoolLockGuard(std::fs::File);

/// Media Pool
pub struct MediaPool {

    name: String,

    media_set_policy: MediaSetPolicy,
    retention: RetentionPolicy,

    inventory: Inventory,
    state_db: MediaStateDatabase,

    current_media_set: MediaSet,
}

impl MediaPool {

    /// Creates a new instance
    pub fn new(
        name: &str,
        state_path: &Path,
        media_set_policy: MediaSetPolicy,
        retention: RetentionPolicy,
     ) -> Result<Self, Error> {

        let inventory = Inventory::load(state_path)?;

        let current_media_set = match inventory.latest_media_set(name) {
            Some(set_uuid) => inventory.compute_media_set_members(&set_uuid)?,
            None => MediaSet::new(),
        };

        let state_db = MediaStateDatabase::load(state_path)?;

        Ok(MediaPool {
            name: String::from(name),
            media_set_policy,
            retention,
            inventory,
            state_db,
            current_media_set,
        })
    }

    /// Creates a new instance using the media pool configuration
    pub fn with_config(
        name: &str,
        state_path: &Path,
        config: &MediaPoolConfig,
    ) -> Result<Self, Error> {

        let allocation = config.allocation.clone().unwrap_or(String::from("continue")).parse()?;

        let retention = config.retention.clone().unwrap_or(String::from("keep")).parse()?;

        MediaPool::new(name, state_path, allocation, retention)
    }

    /// Returns the pool name
    pub fn name(&self) -> &str {
        &self.name
    }

    fn compute_media_state(&self, media_id: &MediaId) -> (MediaStatus, MediaLocation) {

        let (status, location) = self.state_db.status_and_location(&media_id.label.uuid);

        match status {
            MediaStatus::Full | MediaStatus::Damaged | MediaStatus::Retired => {
                return (status, location);
            }
            MediaStatus::Unknown | MediaStatus::Writable => {
                /* possibly writable - fall through to check */
            }
        }

        let set = match media_id.media_set_label {
            None => return (MediaStatus::Writable, location), // not assigned to any pool
            Some(ref set) => set,
        };

        if set.pool != self.name { // should never trigger
            return (MediaStatus::Unknown, location); // belong to another pool
        }
        if set.uuid.as_ref() == [0u8;16] { // not assigned to any pool
            return (MediaStatus::Writable, location);
        }

        if &set.uuid != self.current_media_set.uuid() {
            return (MediaStatus::Full, location); // assume FULL
        }

        // media is member of current set
        if self.current_media_set.is_last_media(&media_id.label.uuid) {
            (MediaStatus::Writable, location) // last set member is writable
        } else {
            (MediaStatus::Full, location)
        }
    }

    /// Returns the 'MediaId' with associated state
    pub fn lookup_media(&self, uuid: &Uuid) -> Result<BackupMedia, Error> {
        let media_id = match self.inventory.lookup_media(uuid) {
            None => bail!("unable to lookup media {}", uuid),
            Some(media_id) => media_id.clone(),
        };

        if let Some(ref set) = media_id.media_set_label {
            if set.pool != self.name {
                bail!("media does not belong to pool ({} != {})", set.pool, self.name);
            }
        }

        let (status, location) = self.compute_media_state(&media_id);

        Ok(BackupMedia::with_media_id(
            media_id,
            location,
            status,
        ))
    }

    /// List all media associated with this pool
    pub fn list_media(&self) -> Vec<BackupMedia> {
        let media_id_list = self.inventory.list_pool_media(&self.name);

        media_id_list.into_iter()
            .map(|media_id| {
                let (status, location) = self.compute_media_state(&media_id);
                BackupMedia::with_media_id(
                    media_id,
                    location,
                    status,
                )
            })
            .collect()
    }

    /// Set media status to FULL.
    pub fn set_media_status_full(&mut self, uuid: &Uuid) -> Result<(), Error> {
        let media = self.lookup_media(uuid)?; // check if media belongs to this pool
        if media.status() != &MediaStatus::Full {
            self.state_db.set_media_status_full(uuid)?;
        }
        Ok(())
    }

    /// Make sure the current media set is usable for writing
    ///
    /// If not, starts a new media set. Also creates a new
    /// set if media_set_policy implies it.
    pub fn start_write_session(&mut self, current_time: i64) -> Result<(), Error> {

        let mut create_new_set = match self.current_set_usable() {
            Err(err) => {
                eprintln!("unable to use current media set - {}", err);
                true
            }
            Ok(usable) => !usable,
        };

        if !create_new_set {

            match &self.media_set_policy {
                MediaSetPolicy::AlwaysCreate => {
                    create_new_set = true;
                }
                MediaSetPolicy::CreateAt(event) => {
                    if let Some(set_start_time) = self.inventory.media_set_start_time(&self.current_media_set.uuid()) {
                        if let Ok(Some(alloc_time)) = compute_next_event(event, set_start_time as i64, false) {
                             if current_time >= alloc_time {
                                create_new_set = true;
                            }
                        }
                    }
                }
                MediaSetPolicy::ContinueCurrent => { /* do nothing here */ }
            }
        }

        if create_new_set {
            let media_set = MediaSet::new();
            eprintln!("starting new media set {}", media_set.uuid());
            self.current_media_set = media_set;
        }

        Ok(())
    }

    /// List media in current media set
    pub fn current_media_list(&self) -> Result<Vec<&Uuid>, Error> {
        let mut list = Vec::new();
        for opt_uuid in self.current_media_set.media_list().iter() {
            match opt_uuid {
                Some(ref uuid) => list.push(uuid),
                None => bail!("current_media_list failed - media set is incomplete"),
            }
        }
        Ok(list)
    }

    // tests if the media data is considered as expired at sepcified time
    pub fn media_is_expired(&self, media: &BackupMedia, current_time: i64) -> bool {
        if media.status() != &MediaStatus::Full {
            return false;
        }

        let expire_time = self.inventory.media_expire_time(
            media.id(), &self.media_set_policy, &self.retention);

        current_time > expire_time
    }

    /// Allocates a writable media to the current media set
    pub fn alloc_writable_media(&mut self, current_time: i64) -> Result<Uuid, Error> {

        let last_is_writable = self.current_set_usable()?;

        let pool = self.name.clone();

        if last_is_writable {
            let last_uuid = self.current_media_set.last_media_uuid().unwrap();
            let media = self.lookup_media(last_uuid)?;
            return Ok(media.uuid().clone());
        }

        // try to find empty media in pool, add to media set

        let mut media_list = self.list_media();

        let mut empty_media = Vec::new();
        for media in media_list.iter_mut() {
            // already part of a media set?
            if media.media_set_label().is_some() { continue; }

            // check if media is on site
            match media.location() {
                MediaLocation::Online(_) | MediaLocation::Offline => { /* OK */ },
                MediaLocation::Vault(_) => continue,
            }

            // only consider writable media
            if media.status() != &MediaStatus::Writable { continue; }

            empty_media.push(media);
        }

        // sort empty_media, oldest media first
        empty_media.sort_unstable_by_key(|media| media.label().ctime);

        if let Some(media) = empty_media.first_mut() {
            // found empty media, add to media set an use it
            let seq_nr = self.current_media_set.media_list().len() as u64;

            let set = MediaSetLabel::with_data(&pool, self.current_media_set.uuid().clone(), seq_nr, current_time);

            media.set_media_set_label(set);

            self.inventory.store(media.id().clone())?; // store persistently

            self.current_media_set.add_media(media.uuid().clone());

            return Ok(media.uuid().clone());
        }

        println!("no empty media in pool, try to reuse expired media");

        let mut expired_media = Vec::new();

        for media in media_list.into_iter() {
            if let Some(set) = media.media_set_label() {
                if &set.uuid == self.current_media_set.uuid() {
                    continue;
                }
            }
            if self.media_is_expired(&media, current_time) {
                println!("found expired media on media '{}'", media.changer_id());
                expired_media.push(media);
            }
        }

        // sort, oldest media first
        expired_media.sort_unstable_by_key(|media| {
            match media.media_set_label() {
                None => 0, // should not happen here
                Some(set) => set.ctime,
            }
        });

        match expired_media.first_mut() {
            None => {
                bail!("alloc writable media in pool '{}' failed: no usable media found", self.name());
            }
            Some(media) => {
                println!("reuse expired media '{}'", media.changer_id());

                let seq_nr = self.current_media_set.media_list().len() as u64;
                let set = MediaSetLabel::with_data(&pool, self.current_media_set.uuid().clone(), seq_nr, current_time);

                media.set_media_set_label(set);

                self.inventory.store(media.id().clone())?; // store persistently
                self.state_db.clear_media_status(media.uuid())?; // remove Full status

                self.current_media_set.add_media(media.uuid().clone());

                return Ok(media.uuid().clone());
            }
        }
    }

    /// check if the current media set is usable for writing
    ///
    /// This does several consistency checks, and return if
    /// the last media in the current set is in writable state.
    ///
    /// This return error when the media set must not be used any
    /// longer because of consistency errors.
    pub fn current_set_usable(&self) -> Result<bool, Error> {

        let media_count = self.current_media_set.media_list().len();
        if media_count == 0 {
            return Ok(false);
        }

        let set_uuid =  self.current_media_set.uuid();
        let mut last_is_writable = false;

        for (seq, opt_uuid) in self.current_media_set.media_list().iter().enumerate() {
            let uuid = match opt_uuid {
                None => bail!("media set is incomplete (missing media information)"),
                Some(uuid) => uuid,
            };
            let media = self.lookup_media(uuid)?;
            match media.media_set_label() {
                Some(MediaSetLabel { seq_nr, uuid, ..}) if *seq_nr == seq as u64 && uuid == set_uuid => { /* OK */ },
                Some(MediaSetLabel { seq_nr, uuid, ..}) if uuid == set_uuid => {
                    bail!("media sequence error ({} != {})", *seq_nr, seq);
                },
                Some(MediaSetLabel { uuid, ..}) => bail!("media owner error ({} != {}", uuid, set_uuid),
                None => bail!("media owner error (no owner)"),
            }
            match media.status() {
                MediaStatus::Full => { /* OK */ },
                MediaStatus::Writable if (seq + 1) == media_count =>  {
                    last_is_writable = true;
                    match media.location() {
                        MediaLocation::Online(_) | MediaLocation::Offline => { /* OK */ },
                        MediaLocation::Vault(vault) => {
                            bail!("writable media offsite in vault '{}'", vault);
                        }
                    }
                },
                _ => bail!("unable to use media set - wrong media status {:?}", media.status()),
            }
        }
        Ok(last_is_writable)
    }

    /// Generate a human readable name for the media set
    pub fn generate_media_set_name(
        &self,
        media_set_uuid: &Uuid,
        template: Option<String>,
    ) -> Result<String, Error> {
        self.inventory.generate_media_set_name(media_set_uuid, template)
    }

    /// Lock the pool
     pub fn lock(base_path: &Path, name: &str) -> Result<MediaPoolLockGuard, Error> {
        let mut path = base_path.to_owned();
        path.push(format!(".{}", name));
        path.set_extension("lck");

        let timeout = std::time::Duration::new(10, 0);
        let lock = proxmox::tools::fs::open_file_locked(&path, timeout, true)?;

        Ok(MediaPoolLockGuard(lock))
    }
}

/// Backup media
///
/// Combines 'MediaId' with 'MediaLocation' and 'MediaStatus'
/// information.
#[derive(Debug,Serialize,Deserialize,Clone)]
pub struct BackupMedia {
    /// Media ID
    id: MediaId,
    /// Media location
    location: MediaLocation,
    /// Media status
    status: MediaStatus,
}

impl BackupMedia {

    /// Creates a new instance
    pub fn with_media_id(
        id: MediaId,
        location: MediaLocation,
        status: MediaStatus,
    ) -> Self {
        Self { id, location, status }
    }

    /// Returns the media location
    pub fn location(&self) -> &MediaLocation {
        &self.location
    }

    /// Returns the media status
    pub fn status(&self) -> &MediaStatus {
        &self.status
    }

    /// Returns the media uuid
    pub fn uuid(&self) -> &Uuid {
        &self.id.label.uuid
    }

    /// Returns the media set label
    pub fn media_set_label(&self) -> &Option<MediaSetLabel> {
        &self.id.media_set_label
    }

    /// Updates the media set label
    pub fn set_media_set_label(&mut self, set_label: MediaSetLabel) {
        self.id.media_set_label = Some(set_label);
    }
    
    /// Returns the drive label
    pub fn label(&self) -> &MediaLabel {
        &self.id.label
    }

    /// Returns the media id (drive label + media set label)
    pub fn id(&self) -> &MediaId {
        &self.id
    }

    /// Returns the media label (Barcode)
    pub fn changer_id(&self) -> &str {
        &self.id.label.changer_id
    }
}
