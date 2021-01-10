mod email;
pub use email::*;

mod parse_mtx_status;
pub use parse_mtx_status::*;

mod mtx_wrapper;
pub use mtx_wrapper::*;

mod mtx;
pub use mtx::*;

use anyhow::{bail, Error};

/// Interface to media change devices
pub trait MediaChange {

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
    /// Note: This refuses to load media inside import/export slots.
    fn load_media(&mut self, changer_id: &str) -> Result<(), Error>;

    /// Unload media from drive
    ///
    /// This is a nop on drives without autoloader.
    fn unload_media(&mut self, target_slot: Option<u64>) -> Result<(), Error>;

    /// Returns true if unload_media automatically ejects drive media
    fn eject_on_unload(&self) -> bool {
        false
    }

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
    fn clean_drive(&mut self) -> Result<(), Error>;

    /// Export media
    ///
    /// By moving the media to an empty import-export slot. Returns
    /// Some(slot) if the media was exported. Returns None if the media is
    /// not online (already exported).
    fn export_media(&mut self, changer_id: &str) -> Result<Option<u64>, Error> {
        let status = self.status()?;

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
