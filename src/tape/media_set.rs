use anyhow::{bail, Error};
use serde::{Deserialize, Serialize};

use proxmox_uuid::Uuid;

/// MediaSet - Ordered group of media
#[derive(Debug, Serialize, Deserialize)]
pub struct MediaSet {
    /// Unique media set ID
    uuid: Uuid,
    /// List of member media IDs
    media_list: Vec<Option<Uuid>>,
}

impl MediaSet {
    pub const MEDIA_SET_MAX_SEQ_NR: u64 = 100;

    #[allow(clippy::new_without_default)]
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
            bail!(
                "media set sequence number to large in media set {} ({} > {})",
                self.uuid.to_string(),
                seq_nr,
                Self::MEDIA_SET_MAX_SEQ_NR
            );
        }
        let seq_nr = seq_nr as usize;
        if self.media_list.len() > seq_nr {
            if self.media_list[seq_nr].is_some() {
                bail!(
                    "found duplicate sequence number in media set '{}/{}'",
                    self.uuid.to_string(),
                    seq_nr
                );
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
