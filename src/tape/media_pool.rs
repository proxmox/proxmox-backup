//! Media Pool
//!
//! A set of backup medias.
//!
//! This struct manages backup media state during backup. The main
//! purpose is to allocate media sets and assign new tapes to it.
//!
//!

use std::path::{PathBuf, Path};
use std::fs::File;

use anyhow::{bail, Error};
use ::serde::{Deserialize, Serialize};

use proxmox::tools::Uuid;

use crate::{
    backup::Fingerprint,
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
        lock_media_set,
        lock_media_pool,
        lock_unassigned_media_pool,
        file_formats::{
            MediaLabel,
            MediaSetLabel,
        },
    }
};

/// Media Pool
pub struct MediaPool {

    name: String,
    state_path: PathBuf,

    media_set_policy: MediaSetPolicy,
    retention: RetentionPolicy,

    changer_name: Option<String>,
    force_media_availability: bool,

    // Set this if you do not need to allocate writeable media -  this
    // is useful for list_media()
    no_media_set_locking: bool,

    encrypt_fingerprint: Option<Fingerprint>,

    inventory: Inventory,

    current_media_set: MediaSet,
    current_media_set_lock: Option<File>,
}

impl MediaPool {

    /// Creates a new instance
    ///
    /// If you specify a `changer_name`, only media accessible via
    /// that changer is considered available.  If you pass `None` for
    /// `changer`, all offline media is considered available (backups
    /// to standalone drives may not use media from inside a tape
    /// library).
    pub fn new(
        name: &str,
        state_path: &Path,
        media_set_policy: MediaSetPolicy,
        retention: RetentionPolicy,
        changer_name: Option<String>,
        encrypt_fingerprint: Option<Fingerprint>,
        no_media_set_locking: bool, // for list_media()
     ) -> Result<Self, Error> {

        let _pool_lock = if no_media_set_locking {
            None
        } else {
            Some(lock_media_pool(state_path, name)?)
        };

        let inventory = Inventory::load(state_path)?;

        let current_media_set = match inventory.latest_media_set(name) {
            Some(set_uuid) => inventory.compute_media_set_members(&set_uuid)?,
            None => MediaSet::new(),
        };

        let current_media_set_lock = if no_media_set_locking {
            None
        } else {
            Some(lock_media_set(state_path, current_media_set.uuid(), None)?)
        };

        Ok(MediaPool {
            name: String::from(name),
            state_path: state_path.to_owned(),
            media_set_policy,
            retention,
            changer_name,
            inventory,
            current_media_set,
            current_media_set_lock,
            encrypt_fingerprint,
            force_media_availability: false,
            no_media_set_locking,
        })
    }

    /// Pretend all Online(x) and Offline media is available
    ///
    /// Only media in Vault(y) is considered unavailable.
    pub fn force_media_availability(&mut self) {
        self.force_media_availability = true;
    }

    /// Returns the the current media set
    pub fn current_media_set(&self) -> &MediaSet {
        &self.current_media_set
    }

    /// Creates a new instance using the media pool configuration
    pub fn with_config(
        state_path: &Path,
        config: &MediaPoolConfig,
        changer_name: Option<String>,
        no_media_set_locking: bool, // for list_media()
    ) -> Result<Self, Error> {

        let allocation = config.allocation.clone().unwrap_or_else(|| String::from("continue")).parse()?;

        let retention = config.retention.clone().unwrap_or_else(|| String::from("keep")).parse()?;

        let encrypt_fingerprint = match config.encrypt {
            Some(ref fingerprint) => Some(fingerprint.parse()?),
            None => None,
        };

        MediaPool::new(
            &config.name,
            state_path,
            allocation,
            retention,
            changer_name,
            encrypt_fingerprint,
            no_media_set_locking,
        )
    }

    /// Returns the pool name
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns encryption settings
    pub fn encrypt_fingerprint(&self) -> Option<Fingerprint> {
        self.encrypt_fingerprint.clone()
    }

    pub fn set_media_status_damaged(&mut self, uuid: &Uuid) -> Result<(), Error> {
        self.inventory.set_media_status_damaged(uuid)
    }

    fn compute_media_state(&self, media_id: &MediaId) -> (MediaStatus, MediaLocation) {

        let (status, location) = self.inventory.status_and_location(&media_id.label.uuid);

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
            self.inventory.set_media_status_full(uuid)?;
        }
        Ok(())
    }

    /// Make sure the current media set is usable for writing
    ///
    /// If not, starts a new media set. Also creates a new
    /// set if media_set_policy implies it.
    ///
    /// Note: We also call this in list_media to compute correct media
    /// status, so this must not change persistent/saved state.
    ///
    /// Returns the reason why we started a new media set (if we do)
    pub fn start_write_session(
        &mut self,
        current_time: i64,
    ) -> Result<Option<String>, Error> {

        let _pool_lock = if self.no_media_set_locking {
            None
        } else {
            Some(lock_media_pool(&self.state_path, &self.name)?)
        };

        self.inventory.reload()?;

        let mut create_new_set = match self.current_set_usable() {
             Err(err) => {
                 Some(err.to_string())
             }
             Ok(_) => None,
        };

        if create_new_set.is_none() {
            match &self.media_set_policy {
                MediaSetPolicy::AlwaysCreate => {
                    create_new_set = Some(String::from("policy is AlwaysCreate"));
                }
                MediaSetPolicy::CreateAt(event) => {
                    if let Some(set_start_time) = self.inventory.media_set_start_time(&self.current_media_set.uuid()) {
                        if let Ok(Some(alloc_time)) = compute_next_event(event, set_start_time as i64, false) {
                            if current_time >= alloc_time {
                                create_new_set = Some(String::from("policy CreateAt event triggered"));
                            }
                        }
                    }
                }
                MediaSetPolicy::ContinueCurrent => { /* do nothing here */ }
            }
        }

        if create_new_set.is_some() {
            let media_set = MediaSet::new();

            let current_media_set_lock = if self.no_media_set_locking {
                None
            } else {
                Some(lock_media_set(&self.state_path, media_set.uuid(), None)?)
            };

            self.current_media_set_lock = current_media_set_lock;
            self.current_media_set = media_set;
        }

        Ok(create_new_set)
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

    // tests if the media data is considered as expired at specified time
    pub fn media_is_expired(&self, media: &BackupMedia, current_time: i64) -> bool {
        if media.status() != &MediaStatus::Full {
            return false;
        }

        let expire_time = self.inventory.media_expire_time(
            media.id(), &self.media_set_policy, &self.retention);

        current_time >= expire_time
    }

    // check if a location is considered on site
    pub fn location_is_available(&self, location: &MediaLocation) -> bool {
        match location {
            MediaLocation::Online(name) => {
                if self.force_media_availability {
                    true
                } else {
                    if let Some(ref changer_name) = self.changer_name {
                        name == changer_name
                    } else {
                        // a standalone drive cannot use media currently inside a library
                        false
                    }
                }
            }
            MediaLocation::Offline => {
                if self.force_media_availability {
                    true
                } else {
                    // consider available for standalone drives
                    self.changer_name.is_none()
                }
            }
            MediaLocation::Vault(_) => false,
        }
    }

    fn add_media_to_current_set(&mut self, mut media_id: MediaId, current_time: i64) -> Result<(), Error> {

        if self.current_media_set_lock.is_none() {
            bail!("add_media_to_current_set: media set is not locked - internal error");
        }

        let seq_nr = self.current_media_set.media_list().len() as u64;

        let pool = self.name.clone();

        let encrypt_fingerprint = self.encrypt_fingerprint();

        let set = MediaSetLabel::with_data(
            &pool,
            self.current_media_set.uuid().clone(),
            seq_nr,
            current_time,
            encrypt_fingerprint,
        );

        media_id.media_set_label = Some(set);

        let uuid = media_id.label.uuid.clone();

        let clear_media_status = true; // remove Full status
        self.inventory.store(media_id, clear_media_status)?; // store persistently

        self.current_media_set.add_media(uuid);

        Ok(())
    }


    /// Allocates a writable media to the current media set
    pub fn alloc_writable_media(&mut self, current_time: i64) -> Result<Uuid, Error> {

        if self.current_media_set_lock.is_none() {
            bail!("alloc_writable_media: media set is not locked - internal error");
        }

        let last_is_writable = self.current_set_usable()?;

        if last_is_writable {
            let last_uuid = self.current_media_set.last_media_uuid().unwrap();
            let media = self.lookup_media(last_uuid)?;
            return Ok(media.uuid().clone());
        }

        // try to find empty media in pool, add to media set

        { // limit pool lock scope
            let _pool_lock = lock_media_pool(&self.state_path, &self.name)?;

            self.inventory.reload()?;

            let media_list = self.list_media();

            let mut empty_media = Vec::new();
            let mut used_media = Vec::new();

            for media in media_list.into_iter() {
                if !self.location_is_available(media.location()) {
                    continue;
                }
                // already part of a media set?
                if media.media_set_label().is_some() {
                    used_media.push(media);
                } else {
                    // only consider writable empty media
                    if media.status() == &MediaStatus::Writable {
                        empty_media.push(media);
                    }
                }
            }

            // sort empty_media, newest first -> oldest last
            empty_media.sort_unstable_by(|a, b| {
                let mut res = b.label().ctime.cmp(&a.label().ctime);
                if res == std::cmp::Ordering::Equal {
                    res = b.label().label_text.cmp(&a.label().label_text);
                }
                res
            });

            if let Some(media) = empty_media.pop() {
                // found empty media, add to media set an use it
                let uuid = media.uuid().clone();
                self.add_media_to_current_set(media.into_id(), current_time)?;
                return Ok(uuid);
            }

            println!("no empty media in pool, try to reuse expired media");

            let mut expired_media = Vec::new();

            for media in used_media.into_iter() {
                if let Some(set) = media.media_set_label() {
                    if &set.uuid == self.current_media_set.uuid() {
                        continue;
                    }
                } else {
                    continue;
                }

                if self.media_is_expired(&media, current_time) {
                    println!("found expired media on media '{}'", media.label_text());
                    expired_media.push(media);
                }
            }

            // sort expired_media, newest first -> oldest last
            expired_media.sort_unstable_by(|a, b| {
                let mut res = b.media_set_label().unwrap().ctime.cmp(&a.media_set_label().unwrap().ctime);
                if res == std::cmp::Ordering::Equal {
                    res = b.label().label_text.cmp(&a.label().label_text);
                }
                res
            });

            while let Some(media) = expired_media.pop() {
                // check if we can modify the media-set (i.e. skip
                // media used by a restore job)
                if let Ok(_media_set_lock) = lock_media_set(
                    &self.state_path,
                    &media.media_set_label().unwrap().uuid,
                    Some(std::time::Duration::new(0, 0)), // do not wait
                ) {
                    println!("reuse expired media '{}'", media.label_text());
                    let uuid = media.uuid().clone();
                    self.add_media_to_current_set(media.into_id(), current_time)?;
                    return Ok(uuid);
                }
            }
        }

        println!("no expired media in pool, try to find unassigned/free media");

        // try unassigned media
        let _lock = lock_unassigned_media_pool(&self.state_path)?;

        self.inventory.reload()?;

        let mut free_media = Vec::new();

        for media_id in self.inventory.list_unassigned_media() {

            let (status, location) = self.compute_media_state(&media_id);
            if media_id.media_set_label.is_some() { continue; } // should not happen

            if !self.location_is_available(&location) {
                continue;
            }

            // only consider writable media
            if status != MediaStatus::Writable { continue; }

            free_media.push(media_id);
        }

        // sort free_media, newest first -> oldest last
        free_media.sort_unstable_by(|a, b| {
            let mut res = b.label.ctime.cmp(&a.label.ctime);
            if res == std::cmp::Ordering::Equal {
                res = b.label.label_text.cmp(&a.label.label_text);
            }
            res
        });

        if let Some(media_id) = free_media.pop() {
            println!("use free media '{}'", media_id.label.label_text);
            let uuid = media_id.label.uuid.clone();
            self.add_media_to_current_set(media_id, current_time)?;
            return Ok(uuid);
        }

        bail!("alloc writable media in pool '{}' failed: no usable media found", self.name());
    }

    /// check if the current media set is usable for writing
    ///
    /// This does several consistency checks, and return if
    /// the last media in the current set is in writable state.
    ///
    /// This return error when the media set must not be used any
    /// longer because of consistency errors.
    pub fn current_set_usable(&self) -> Result<bool, Error> {

        let media_list = self.current_media_set.media_list();

        let media_count = media_list.len();
        if media_count == 0 {
            return Ok(false);
        }

        let set_uuid =  self.current_media_set.uuid();
        let mut last_is_writable = false;

        let mut last_enc: Option<Option<Fingerprint>> = None;

        for (seq, opt_uuid) in media_list.iter().enumerate() {
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

            if let Some(set) = media.media_set_label() { // always true here
                if set.encryption_key_fingerprint != self.encrypt_fingerprint {
                    bail!("pool encryption key changed");
                }
                match last_enc {
                    None => {
                        last_enc = Some(set.encryption_key_fingerprint.clone());
                    }
                    Some(ref last_enc) => {
                        if last_enc != &set.encryption_key_fingerprint {
                            bail!("inconsistent media encryption key");
                        }
                    }
                }
            }

            match media.status() {
                MediaStatus::Full => { /* OK */ },
                MediaStatus::Writable if (seq + 1) == media_count =>  {
                    let media_location = media.location();
                    if self.location_is_available(media_location) {
                        last_is_writable = true;
                    } else {
                        if let MediaLocation::Vault(vault) = media_location {
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
    pub fn media_set_label(&self) -> Option<&MediaSetLabel> {
        self.id.media_set_label.as_ref()
    }

    /// Returns the media creation time
    pub fn ctime(&self) -> i64 {
        self.id.label.ctime
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

    /// Returns the media id, consumes self)
    pub fn into_id(self) -> MediaId {
        self.id
    }

    /// Returns the media label (Barcode)
    pub fn label_text(&self) -> &str {
        &self.id.label.label_text
    }
}
