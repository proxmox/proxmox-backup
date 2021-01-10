use anyhow::{bail, Error};

use crate::{
    tape::changer::{
        MediaChange,
        MtxStatus,
        ElementStatus,
        mtx_status,
        mtx_load,
        mtx_unload,
    },
    api2::types::{
        ScsiTapeChanger,
        LinuxTapeDrive,
    },
};

/// Implements MediaChange using 'mtx' linux cli tool
pub struct MtxMediaChanger {
    drive_name: String, // used for error messages
    drivenum: u64,
    config: ScsiTapeChanger,
}

impl MtxMediaChanger {

    pub fn with_drive_config(drive_config: &LinuxTapeDrive) -> Result<Self, Error> {
        let (config, _digest) = crate::config::drive::config()?;
        let changer_config: ScsiTapeChanger = match drive_config.changer {
            Some(ref changer) => config.lookup("changer", changer)?,
            None => bail!("drive '{}' has no associated changer", drive_config.name),
        };

        Ok(Self {
            drive_name: drive_config.name.clone(),
            drivenum: drive_config.changer_drive_id.unwrap_or(0),
            config: changer_config,
        })
    }
}

fn unload_to_free_slot(drive_name: &str, path: &str, status: &MtxStatus, drivenum: u64) -> Result<(), Error> {

    if drivenum >= status.drives.len() as u64 {
        bail!("unload drive '{}' got unexpected drive number '{}' - changer only has '{}' drives",
              drive_name, drivenum, status.drives.len());
    }
    let drive_status = &status.drives[drivenum as usize];
    if let Some(slot) = drive_status.loaded_slot {
        mtx_unload(path, slot, drivenum)
    } else {
        let mut free_slot = None;
        for i in 0..status.slots.len() {
            if status.slots[i].0 { continue; } // skip import/export slots
            if let ElementStatus::Empty = status.slots[i].1 {
                free_slot = Some((i+1) as u64);
                break;
            }
        }
        if let Some(slot) = free_slot {
            mtx_unload(path, slot, drivenum)
        } else {
            bail!("drive '{}' unload failure - no free slot", drive_name);
        }
    }
}

impl MediaChange for MtxMediaChanger {

    fn status(&mut self) -> Result<MtxStatus, Error> {
        mtx_status(&self.config)
    }

    fn load_media_from_slot(&mut self, slot: u64) -> Result<(), Error> {
        mtx_load(&self.config.path, slot, self.drivenum)
    }

    fn load_media(&mut self, changer_id: &str) -> Result<(), Error> {

        if changer_id.starts_with("CLN") {
            bail!("unable to load media '{}' (seems top be a a cleaning units)", changer_id);
        }

        let status = self.status()?;

         // already loaded?
        for (i, drive_status) in status.drives.iter().enumerate() {
            if let ElementStatus::VolumeTag(ref tag) = drive_status.status {
                if *tag == changer_id {
                    if i as u64 != self.drivenum {
                        bail!("unable to load media '{}' - media in wrong drive ({} != {})",
                              changer_id, i, self.drivenum);
                    }
                    return Ok(())
                }
            }
            if i as u64 == self.drivenum {
                match drive_status.status {
                    ElementStatus::Empty => { /* OK */ },
                    _ => unload_to_free_slot(&self.drive_name, &self.config.path, &status, self.drivenum)?,
                }
            }
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

        mtx_load(&self.config.path, slot as u64, self.drivenum)
    }

    fn unload_media(&mut self, target_slot: Option<u64>) -> Result<(), Error> {
        if let Some(target_slot) = target_slot {
            mtx_unload(&self.config.path, target_slot, self.drivenum)
        } else {
            let status = self.status()?;
            unload_to_free_slot(&self.drive_name, &self.config.path, &status, self.drivenum)
        }
    }

    fn eject_on_unload(&self) -> bool {
        true
    }

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

        if let Some(drive_status) = status.drives.get(self.drivenum as usize) {
            match drive_status.status {
                ElementStatus::Empty => { /* OK */ },
                _ => unload_to_free_slot(&self.drive_name, &self.config.path, &status, self.drivenum)?,
            }
        }

        mtx_load(&self.config.path, cleaning_cartridge_slot, self.drivenum)?;

        mtx_unload(&self.config.path, cleaning_cartridge_slot, self.drivenum)?;

        Ok(())
    }
}
