mod email;
pub use email::*;

mod parse_mtx_status;
pub use parse_mtx_status::*;

mod mtx_wrapper;
pub use mtx_wrapper::*;

mod linux_tape;
pub use linux_tape::*;

use anyhow::Error;

/// Interface to media change devices
pub trait MediaChange {

    /// Returns the changer status
    fn status(&mut self) -> Result<MtxStatus, Error>;

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
}
