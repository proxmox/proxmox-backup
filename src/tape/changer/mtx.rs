use anyhow::{bail, Error};

use crate::{
    tape::changer::{
        MediaChange,
        MtxStatus,
        mtx_status,
        mtx_transfer,
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
    drive_number: u64,
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
            drive_number: drive_config.changer_drive_id.unwrap_or(0),
            config: changer_config,
        })
    }
}

impl MediaChange for MtxMediaChanger {

    fn drive_number(&self) -> u64 {
        self.drive_number
    }

    fn drive_name(&self) -> &str {
        &self.drive_name
    }

    fn status(&mut self) -> Result<MtxStatus, Error> {
        mtx_status(&self.config)
    }

    fn transfer_media(&mut self, from: u64, to: u64) -> Result<(), Error> {
        mtx_transfer(&self.config.path, from, to)
    }

    fn load_media_from_slot(&mut self, slot: u64) -> Result<(), Error> {
        mtx_load(&self.config.path, slot, self.drive_number)
    }

    fn unload_media(&mut self, target_slot: Option<u64>) -> Result<(), Error> {
        if let Some(target_slot) = target_slot {
            mtx_unload(&self.config.path, target_slot, self.drive_number)
        } else {
            let status = self.status()?;
            self.unload_to_free_slot(status)
        }
    }

    fn eject_on_unload(&self) -> bool {
        true
    }
}
