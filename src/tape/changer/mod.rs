//! Media changer implementation (SCSI media changer)

pub mod mtx;

mod online_status_map;
pub use online_status_map::*;

use std::path::PathBuf;

use anyhow::{bail, Error};

use proxmox_sys::fs::{file_read_optional_string, replace_file, CreateOptions};

use pbs_api_types::{LtoTapeDrive, ScsiTapeChanger};

use pbs_tape::{linux_list_drives::open_lto_tape_device, sg_pt_changer, ElementStatus, MtxStatus};

use crate::tape::drive::{LtoTapeHandle, TapeDriver};

/// Interface to SCSI changer devices
pub trait ScsiMediaChange {
    fn status(&mut self, use_cache: bool) -> Result<MtxStatus, Error>;

    fn load_slot(&mut self, from_slot: u64, drivenum: u64) -> Result<MtxStatus, Error>;

    fn unload(&mut self, to_slot: u64, drivenum: u64) -> Result<MtxStatus, Error>;

    fn transfer(&mut self, from_slot: u64, to_slot: u64) -> Result<MtxStatus, Error>;
}

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
    fn transfer_media(&mut self, from: u64, to: u64) -> Result<MtxStatus, Error>;

    /// Load media from storage slot into drive
    fn load_media_from_slot(&mut self, slot: u64) -> Result<MtxStatus, Error>;

    /// Load media by label-text into drive
    ///
    /// This unloads first if the drive is already loaded with another media.
    ///
    /// Note: This refuses to load media inside import/export
    /// slots. Also, you cannot load cleaning units with this
    /// interface.
    fn load_media(&mut self, label_text: &str) -> Result<MtxStatus, Error> {
        if label_text.starts_with("CLN") {
            bail!(
                "unable to load media '{}' (seems to be a cleaning unit)",
                label_text
            );
        }

        let mut status = self.status()?;

        let mut unload_drive = false;

        // already loaded?
        for (i, drive_status) in status.drives.iter().enumerate() {
            if let ElementStatus::VolumeTag(ref tag) = drive_status.status {
                if *tag == label_text {
                    if i as u64 != self.drive_number() {
                        bail!(
                            "unable to load media '{}' - media in wrong drive ({} != {})",
                            label_text,
                            i,
                            self.drive_number()
                        );
                    }
                    return Ok(status); // already loaded
                }
            }
            if i as u64 == self.drive_number() {
                match drive_status.status {
                    ElementStatus::Empty => { /* OK */ }
                    _ => unload_drive = true,
                }
            }
        }

        if unload_drive {
            status = self.unload_to_free_slot(status)?;
        }

        let mut slot = None;
        for (i, slot_info) in status.slots.iter().enumerate() {
            if let ElementStatus::VolumeTag(ref tag) = slot_info.status {
                if tag == label_text {
                    if slot_info.import_export {
                        bail!(
                            "unable to load media '{}' - inside import/export slot",
                            label_text
                        );
                    }
                    slot = Some(i + 1);
                    break;
                }
            }
        }

        let slot = match slot {
            None => bail!("unable to find media '{}' (offline?)", label_text),
            Some(slot) => slot,
        };

        self.load_media_from_slot(slot as u64)
    }

    /// Unload media from drive (eject media if necessary)
    fn unload_media(&mut self, target_slot: Option<u64>) -> Result<MtxStatus, Error>;

    /// List online media labels (label_text/barcodes)
    ///
    /// List accessible (online) label texts. This does not include
    /// media inside import-export slots or cleaning media.
    fn online_media_label_texts(&mut self) -> Result<Vec<String>, Error> {
        let status = self.status()?;

        let mut list = Vec::new();

        for drive_status in status.drives.iter() {
            if let ElementStatus::VolumeTag(ref tag) = drive_status.status {
                list.push(tag.clone());
            }
        }

        for slot_info in status.slots.iter() {
            if slot_info.import_export {
                continue;
            }
            if let ElementStatus::VolumeTag(ref tag) = slot_info.status {
                if tag.starts_with("CLN") {
                    continue;
                }
                list.push(tag.clone());
            }
        }

        Ok(list)
    }

    /// Load/Unload cleaning cartridge
    ///
    /// This fail if there is no cleaning cartridge online. Any media
    /// inside the drive is automatically unloaded.
    fn clean_drive(&mut self) -> Result<MtxStatus, Error> {
        let mut status = self.status()?;

        // Unload drive first. Note: This also unloads a loaded cleaning tape
        if let Some(drive_status) = status.drives.get(self.drive_number() as usize) {
            match drive_status.status {
                ElementStatus::Empty => { /* OK */ }
                _ => {
                    status = self.unload_to_free_slot(status)?;
                }
            }
        }

        let mut cleaning_cartridge_slot = None;

        for (i, slot_info) in status.slots.iter().enumerate() {
            if slot_info.import_export {
                continue;
            }
            if let ElementStatus::VolumeTag(ref tag) = slot_info.status {
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

        self.load_media_from_slot(cleaning_cartridge_slot)?;

        self.unload_media(Some(cleaning_cartridge_slot))
    }

    /// Export media
    ///
    /// By moving the media to an empty import-export slot. Returns
    /// Some(slot) if the media was exported. Returns None if the media is
    /// not online (already exported).
    fn export_media(&mut self, label_text: &str) -> Result<Option<u64>, Error> {
        let status = self.status()?;

        let mut unload_from_drive = false;
        if let Some(drive_status) = status.drives.get(self.drive_number() as usize) {
            if let ElementStatus::VolumeTag(ref tag) = drive_status.status {
                if tag == label_text {
                    unload_from_drive = true;
                }
            }
        }

        let mut from = None;
        let mut to = None;

        for (i, slot_info) in status.slots.iter().enumerate() {
            if slot_info.import_export {
                if to.is_some() {
                    continue;
                }
                if let ElementStatus::Empty = slot_info.status {
                    to = Some(i as u64 + 1);
                }
            } else if let ElementStatus::VolumeTag(ref tag) = slot_info.status {
                if tag == label_text {
                    from = Some(i as u64 + 1);
                }
            }
        }

        if unload_from_drive {
            match to {
                Some(to) => {
                    self.unload_media(Some(to))?;
                    Ok(Some(to))
                }
                None => bail!("unable to find free export slot"),
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
    /// If possible to the slot it was previously loaded from.
    ///
    /// Note: This method consumes status - so please use returned status afterward.
    fn unload_to_free_slot(&mut self, status: MtxStatus) -> Result<MtxStatus, Error> {
        let drive_status = &status.drives[self.drive_number() as usize];
        if let Some(slot) = drive_status.loaded_slot {
            // check if original slot is empty/usable
            if let Some(info) = status.slots.get(slot as usize - 1) {
                if let ElementStatus::Empty = info.status {
                    return self.unload_media(Some(slot));
                }
            }
        }

        if let Some(slot) = status.find_free_slot(false) {
            self.unload_media(Some(slot))
        } else {
            bail!(
                "drive '{}' unload failure - no free slot",
                self.drive_name()
            );
        }
    }
}

const USE_MTX: bool = false;

impl ScsiMediaChange for ScsiTapeChanger {
    fn status(&mut self, use_cache: bool) -> Result<MtxStatus, Error> {
        if use_cache {
            if let Some(state) = load_changer_state_cache(&self.name)? {
                return Ok(state);
            }
        }

        let status = if USE_MTX {
            mtx::mtx_status(self)
        } else {
            sg_pt_changer::status(self)
        };

        match &status {
            Ok(status) => {
                save_changer_state_cache(&self.name, status)?;
            }
            Err(_) => {
                delete_changer_state_cache(&self.name);
            }
        }

        status
    }

    fn load_slot(&mut self, from_slot: u64, drivenum: u64) -> Result<MtxStatus, Error> {
        let result = if USE_MTX {
            mtx::mtx_load(&self.path, from_slot, drivenum)
        } else {
            let mut file = sg_pt_changer::open(&self.path)?;
            sg_pt_changer::load_slot(&mut file, from_slot, drivenum)
        };

        let status = self.status(false)?; // always update status

        result?; // check load result

        Ok(status)
    }

    fn unload(&mut self, to_slot: u64, drivenum: u64) -> Result<MtxStatus, Error> {
        let result = if USE_MTX {
            mtx::mtx_unload(&self.path, to_slot, drivenum)
        } else {
            let mut file = sg_pt_changer::open(&self.path)?;
            sg_pt_changer::unload(&mut file, to_slot, drivenum)
        };

        let status = self.status(false)?; // always update status

        result?; // check unload result

        Ok(status)
    }

    fn transfer(&mut self, from_slot: u64, to_slot: u64) -> Result<MtxStatus, Error> {
        let result = if USE_MTX {
            mtx::mtx_transfer(&self.path, from_slot, to_slot)
        } else {
            let mut file = sg_pt_changer::open(&self.path)?;
            sg_pt_changer::transfer_medium(&mut file, from_slot, to_slot)
        };

        let status = self.status(false)?; // always update status

        result?; // check unload result

        Ok(status)
    }
}

fn save_changer_state_cache(changer: &str, state: &MtxStatus) -> Result<(), Error> {
    let mut path = PathBuf::from(crate::tape::CHANGER_STATE_DIR);
    path.push(changer);

    let state = serde_json::to_string_pretty(state)?;

    let backup_user = pbs_config::backup_user()?;
    let mode = nix::sys::stat::Mode::from_bits_truncate(0o0644);
    let options = CreateOptions::new()
        .perm(mode)
        .owner(backup_user.uid)
        .group(backup_user.gid);

    replace_file(path, state.as_bytes(), options, false)
}

fn delete_changer_state_cache(changer: &str) {
    let mut path = PathBuf::from("/run/proxmox-backup/changer-state");
    path.push(changer);

    let _ = std::fs::remove_file(&path); // ignore errors
}

fn load_changer_state_cache(changer: &str) -> Result<Option<MtxStatus>, Error> {
    let mut path = PathBuf::from("/run/proxmox-backup/changer-state");
    path.push(changer);

    let data = match file_read_optional_string(&path)? {
        None => return Ok(None),
        Some(data) => data,
    };

    let state = serde_json::from_str(&data)?;

    Ok(Some(state))
}

/// Implements MediaChange using 'mtx' linux cli tool
pub struct MtxMediaChanger {
    drive: LtoTapeDrive,
    config: ScsiTapeChanger,
}

impl MtxMediaChanger {
    pub fn with_drive_config(drive_config: &LtoTapeDrive) -> Result<Self, Error> {
        let (config, _digest) = pbs_config::drive::config()?;
        let changer_config: ScsiTapeChanger = match drive_config.changer {
            Some(ref changer) => config.lookup("changer", changer)?,
            None => bail!("drive '{}' has no associated changer", drive_config.name),
        };

        Ok(Self {
            drive: drive_config.clone(),
            config: changer_config,
        })
    }
}

impl MediaChange for MtxMediaChanger {
    fn drive_number(&self) -> u64 {
        self.drive.changer_drivenum.unwrap_or(0)
    }

    fn drive_name(&self) -> &str {
        &self.drive.name
    }

    fn status(&mut self) -> Result<MtxStatus, Error> {
        self.config.status(false)
    }

    fn transfer_media(&mut self, from: u64, to: u64) -> Result<MtxStatus, Error> {
        self.config.transfer(from, to)
    }

    fn load_media_from_slot(&mut self, slot: u64) -> Result<MtxStatus, Error> {
        self.config.load_slot(slot, self.drive_number())
    }

    fn unload_media(&mut self, target_slot: Option<u64>) -> Result<MtxStatus, Error> {
        if self.config.eject_before_unload.unwrap_or(false) {
            let file = open_lto_tape_device(&self.drive.path)?;
            let mut handle = LtoTapeHandle::new(file)?;

            if handle.medium_present() {
                handle.eject_media()?;
            }
        }

        if let Some(target_slot) = target_slot {
            self.config.unload(target_slot, self.drive_number())
        } else {
            let status = self.status()?;
            self.unload_to_free_slot(status)
        }
    }
}
