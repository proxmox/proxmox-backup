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

impl MediaChange for LinuxTapeDrive {

    fn load_media(&mut self, changer_id: &str) -> Result<(), Error> {

        if changer_id.starts_with("CLN") {
            bail!("unable to load media '{}' (seems top be a a cleaning units)", changer_id);
        }

        let (config, _digest) = crate::config::drive::config()?;

        let changer: ScsiTapeChanger = match self.changer {
            Some(ref changer) => config.lookup("changer", changer)?,
            None => bail!("drive '{}' has no associated changer", self.name),
        };

        let status = mtx_status(&changer.path)?;

        let drivenum = self.changer_drive_id.unwrap_or(0);

        // already loaded?
        for (i, drive_status) in status.drives.iter().enumerate() {
            if let ElementStatus::VolumeTag(ref tag) = drive_status.status {
                if *tag == changer_id {
                    if i as u64 != drivenum {
                        bail!("unable to load media '{}' - media in wrong drive ({} != {})",
                              changer_id, i, drivenum);
                    }
                    return Ok(())
                }
            }
            if i as u64 == drivenum {
                match  drive_status.status {
                    ElementStatus::Empty => { /* OK */ },
                    _ => unload_to_free_slot(&self.name, &changer.path, &status, drivenum as u64)?,
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


        mtx_load(&changer.path, slot as u64, drivenum as u64)
    }

    fn unload_media(&mut self) -> Result<(), Error> {
        let (config, _digest) = crate::config::drive::config()?;

        let changer: ScsiTapeChanger = match self.changer {
            Some(ref changer) => config.lookup("changer", changer)?,
            None => return Ok(()),
        };

        let drivenum = self.changer_drive_id.unwrap_or(0);

        let status = mtx_status(&changer.path)?;

        unload_to_free_slot(&self.name, &changer.path, &status, drivenum)
    }

    fn eject_on_unload(&self) -> bool {
        true
    }

    fn list_media_changer_ids(&self) -> Result<Vec<String>, Error> {
        let (config, _digest) = crate::config::drive::config()?;

        let changer: ScsiTapeChanger = match self.changer {
            Some(ref changer) => config.lookup("changer", changer)?,
            None => return Ok(Vec::new()),
        };

        let status = mtx_status(&changer.path)?;

        let mut list = Vec::new();

        for drive_status in status.drives.iter() {
            if let ElementStatus::VolumeTag(ref tag) = drive_status.status {
                list.push(tag.clone());
            }
        }

        for (import_export, element_status) in status.slots.iter() {
            if *import_export { continue; }
            if let ElementStatus::VolumeTag(ref tag) = element_status {
                list.push(tag.clone());
            }
        }

        Ok(list)
    }
}
