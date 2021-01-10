mod email;
pub use email::*;

mod parse_mtx_status;
pub use parse_mtx_status::*;

mod mtx_wrapper;
pub use mtx_wrapper::*;

mod mtx;
pub use mtx::*;

use anyhow::{bail, Error};

/// Interface to the media changer device for a single drive
pub trait MediaChange {

    /// Drive number inside changer
    fn drive_number(&self) -> u64;

    /// Drive name (used for debug messages)
    fn drive_name(&self) -> &str;

    /// Returns the changer status
    fn status(&mut self) -> Result<MtxStatus, Error>;

    /// Transfer media from on slot to another (storage or import export slots)
    ///
    /// Target slot needs to be empty
    fn transfer_media(&mut self, from: u64, to: u64) -> Result<(), Error>;

    /// Load media from storage slot into drive
    fn load_media_from_slot(&mut self, slot: u64) -> Result<(), Error>;

    /// Load media by changer-id into drive
    ///
    /// This unloads first if the drive is already loaded with another media.
    ///
    /// Note: This refuses to load media inside import/export
    /// slots. Also, you cannot load cleaning units with this
    /// interface.
    fn load_media(&mut self, changer_id: &str) -> Result<(), Error> {

        if changer_id.starts_with("CLN") {
            bail!("unable to load media '{}' (seems top be a a cleaning units)", changer_id);
        }

        let mut status = self.status()?;

        let mut unload_drive = false;

        // already loaded?
        for (i, drive_status) in status.drives.iter().enumerate() {
            if let ElementStatus::VolumeTag(ref tag) = drive_status.status {
                if *tag == changer_id {
                    if i as u64 != self.drive_number() {
                        bail!("unable to load media '{}' - media in wrong drive ({} != {})",
                              changer_id, i, self.drive_number());
                    }
                    return Ok(()) // already loaded
                }
            }
            if i as u64 == self.drive_number() {
                match drive_status.status {
                    ElementStatus::Empty => { /* OK */ },
                    _ => unload_drive = true,
                 }
            }
        }

        if unload_drive {
            self.unload_to_free_slot(status)?;
            status = self.status()?;
        }

        let mut slot = None;
        for (i, (import_export, element_status)) in status.slots.iter().enumerate() {
            if let ElementStatus::VolumeTag(tag) = element_status {
                if *tag == changer_id {
                    if *import_export {
                        bail!("unable to load media '{}' - inside import/export slot", changer_id);
                    }
                    slot = Some(i+1);
                    break;
                }
            }
        }

        let slot = match slot {
            None => bail!("unable to find media '{}' (offline?)", changer_id),
            Some(slot) => slot,
        };

        self.load_media_from_slot(slot as u64)
    }

    /// Unload media from drive (eject media if necessary)
    fn unload_media(&mut self, target_slot: Option<u64>) -> Result<(), Error>;

    /// List online media changer IDs (barcodes)
    ///
    /// List acessible (online) changer IDs. This does not include
    /// media inside import-export slots or cleaning media.
    fn online_media_changer_ids(&mut self) -> Result<Vec<String>, Error> {
        let status = self.status()?;

        let mut list = Vec::new();

        for drive_status in status.drives.iter() {
            if let ElementStatus::VolumeTag(ref tag) = drive_status.status {
                list.push(tag.clone());
            }
        }

        for (import_export, element_status) in status.slots.iter() {
            if *import_export { continue; }
            if let ElementStatus::VolumeTag(ref tag) = element_status {
                if tag.starts_with("CLN") { continue; }
                list.push(tag.clone());
            }
        }

        Ok(list)
    }

    /// Load/Unload cleaning cartridge
    ///
    /// This fail if there is no cleaning cartridge online. Any media
    /// inside the drive is automatically unloaded.
    fn clean_drive(&mut self) -> Result<(), Error> {
        let status = self.status()?;

        let mut cleaning_cartridge_slot = None;

        for (i, (import_export, element_status)) in status.slots.iter().enumerate() {
            if *import_export { continue; }
            if let ElementStatus::VolumeTag(ref tag) = element_status {
                if tag.starts_with("CLN") {
                    cleaning_cartridge_slot = Some(i + 1);
                    break;
                }
            }
        }

        let cleaning_cartridge_slot = match cleaning_cartridge_slot {
            None => bail!("clean failed - unable to find cleaning cartridge"),
            Some(cleaning_cartridge_slot) => cleaning_cartridge_slot as u64,
        };

        if let Some(drive_status) = status.drives.get(self.drive_number() as usize) {
            match drive_status.status {
                ElementStatus::Empty => { /* OK */ },
                _ => self.unload_to_free_slot(status)?,
            }
        }

        self.load_media_from_slot(cleaning_cartridge_slot)?;

        self.unload_media(Some(cleaning_cartridge_slot))?;

        Ok(())
    }

    /// Export media
    ///
    /// By moving the media to an empty import-export slot. Returns
    /// Some(slot) if the media was exported. Returns None if the media is
    /// not online (already exported).
    fn export_media(&mut self, changer_id: &str) -> Result<Option<u64>, Error> {
        let status = self.status()?;

        let mut unload_from_drive = false;
        if let Some(drive_status) = status.drives.get(self.drive_number() as usize) {
            if let ElementStatus::VolumeTag(ref tag) = drive_status.status {
                if tag == changer_id {
                    unload_from_drive = true;
                }
            }
        }

        let mut from = None;
        let mut to = None;

        for (i, (import_export, element_status)) in status.slots.iter().enumerate() {
            if *import_export {
                if to.is_some() { continue; }
                if let ElementStatus::Empty = element_status {
                    to = Some(i as u64 + 1);
                }
            } else {
                if let ElementStatus::VolumeTag(ref tag) = element_status {
                    if tag == changer_id {
                        from = Some(i as u64 + 1);
                    }
                }
            }
        }

        if unload_from_drive {
            match to {
                Some(to) => {
                    self.unload_media(Some(to))?;
                    Ok(Some(to))
                }
                None =>  bail!("unable to find free export slot"),
            }
        } else {
            match (from, to) {
                (Some(from), Some(to)) => {
                    self.transfer_media(from, to)?;
                    Ok(Some(to))
                }
                (Some(_from), None) => bail!("unable to find free export slot"),
                (None, _) => Ok(None), // not online
            }
        }
    }

    /// Unload media to a free storage slot
    ///
    /// If posible to the slot it was previously loaded from.
    ///
    /// Note: This method consumes status - so please read again afterward.
    fn unload_to_free_slot(&mut self, status: MtxStatus) -> Result<(), Error> {

        let drive_status = &status.drives[self.drive_number() as usize];
        if let Some(slot) = drive_status.loaded_slot {
            // check if original slot is empty/usable
            if let Some(info) = status.slots.get(slot as usize - 1) {
                if let (_import_export, ElementStatus::Empty) = info {
                    return self.unload_media(Some(slot));
                }
            }
        }

        let mut free_slot = None;
        for i in 0..status.slots.len() {
            if status.slots[i].0 { continue; } // skip import/export slots
            if let ElementStatus::Empty = status.slots[i].1 {
                free_slot = Some((i+1) as u64);
                break;
            }
        }
        if let Some(slot) = free_slot {
            self.unload_media(Some(slot))
        } else {
            bail!("drive '{}' unload failure - no free slot", self.drive_name());
        }
    }
}
