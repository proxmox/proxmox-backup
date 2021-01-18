mod virtual_tape;
mod linux_mtio;

mod tape_alert_flags;
pub use tape_alert_flags::*;

mod volume_statistics;
pub use volume_statistics::*;

mod encryption;
pub use encryption::*;

pub mod linux_tape;

mod mam;
pub use mam::*;

mod linux_list_drives;
pub use linux_list_drives::*;

use anyhow::{bail, format_err, Error};
use ::serde::{Deserialize};

use proxmox::tools::io::ReadExt;
use proxmox::api::section_config::SectionConfigData;

use crate::{
    backup::Fingerprint,
    api2::types::{
        VirtualTapeDrive,
        LinuxTapeDrive,
    },
    server::WorkerTask,
    tape::{
        TapeWrite,
        TapeRead,
        MediaId,
        MtxMediaChanger,
        file_formats::{
            PROXMOX_BACKUP_MEDIA_LABEL_MAGIC_1_0,
            PROXMOX_BACKUP_MEDIA_SET_LABEL_MAGIC_1_0,
            MediaLabel,
            MediaSetLabel,
            MediaContentHeader,
        },
        changer::{
            MediaChange,
            send_load_media_email,
        },
    },
};

/// Tape driver interface
pub trait TapeDriver {

    /// Flush all data to the tape
    fn sync(&mut self) -> Result<(), Error>;

    /// Rewind the tape
    fn rewind(&mut self) -> Result<(), Error>;

    /// Move to end of recorded data
    ///
    /// We assume this flushes the tape write buffer.
    fn move_to_eom(&mut self) -> Result<(), Error>;

    /// Current file number
    fn current_file_number(&mut self) -> Result<u64, Error>;

    /// Completely erase the media
    fn erase_media(&mut self, fast: bool) -> Result<(), Error>;

    /// Read/Open the next file
    fn read_next_file<'a>(&'a mut self) -> Result<Option<Box<dyn TapeRead + 'a>>, std::io::Error>;

    /// Write/Append a new file
    fn write_file<'a>(&'a mut self) -> Result<Box<dyn TapeWrite + 'a>, std::io::Error>;

    /// Write label to tape (erase tape content)
    fn label_tape(&mut self, label: &MediaLabel) -> Result<(), Error> {

        self.rewind()?;

        self.erase_media(true)?;

        let raw = serde_json::to_string_pretty(&serde_json::to_value(&label)?)?;

        let header = MediaContentHeader::new(PROXMOX_BACKUP_MEDIA_LABEL_MAGIC_1_0, raw.len() as u32);

        {
            let mut writer = self.write_file()?;
            writer.write_header(&header, raw.as_bytes())?;
            writer.finish(false)?;
        }

        self.sync()?; // sync data to tape

        Ok(())
    }

    /// Write the media set label to tape
    fn write_media_set_label(&mut self, media_set_label: &MediaSetLabel) -> Result<(), Error>;

    /// Read the media label
    ///
    /// This tries to read both media labels (label and media_set_label).
    fn read_label(&mut self) -> Result<Option<MediaId>, Error> {

        self.rewind()?;

        let label = {
            let mut reader = match self.read_next_file()? {
                None => return Ok(None), // tape is empty
                Some(reader) => reader,
            };

            let header: MediaContentHeader = unsafe { reader.read_le_value()? };
            header.check(PROXMOX_BACKUP_MEDIA_LABEL_MAGIC_1_0, 1, 64*1024)?;
            let data = reader.read_exact_allocated(header.size as usize)?;

            let label: MediaLabel = serde_json::from_slice(&data)
                .map_err(|err| format_err!("unable to parse drive label - {}", err))?;

            // make sure we read the EOF marker
            if reader.skip_to_end()? != 0 {
                bail!("got unexpected data after label");
            }

            label
        };

        let mut media_id = MediaId { label, media_set_label: None };

        // try to read MediaSet label
        let mut reader = match self.read_next_file()? {
            None => return Ok(Some(media_id)),
            Some(reader) => reader,
        };

        let header: MediaContentHeader = unsafe { reader.read_le_value()? };
        header.check(PROXMOX_BACKUP_MEDIA_SET_LABEL_MAGIC_1_0, 1, 64*1024)?;
        let data = reader.read_exact_allocated(header.size as usize)?;

        let media_set_label: MediaSetLabel = serde_json::from_slice(&data)
            .map_err(|err| format_err!("unable to parse media set label - {}", err))?;

        // make sure we read the EOF marker
        if reader.skip_to_end()? != 0 {
            bail!("got unexpected data after media set label");
        }

        media_id.media_set_label = Some(media_set_label);

        Ok(Some(media_id))
    }

    /// Eject media
    fn eject_media(&mut self) -> Result<(), Error>;

    /// Read Tape Alert Flags
    ///
    /// This make only sense for real LTO drives. Virtual tape drives should
    /// simply return empty flags (default).
    fn tape_alert_flags(&mut self) -> Result<TapeAlertFlags, Error> {
        Ok(TapeAlertFlags::empty())
    }

    /// Set or clear encryption key
    fn set_encryption(&mut self, key_fingerprint: Option<Fingerprint>) -> Result<(), Error> {
        if key_fingerprint.is_some() {
            bail!("drive does not support encryption");
        }
        Ok(())
    }
}

/// Get the media changer (MediaChange + name) associated with a tape drive.
///
/// Returns Ok(None) if the drive has no associated changer device.
///
/// Note: This may return the drive name as changer-name if the drive
/// implements some kind of internal changer (which is true for our
/// 'virtual' drive implementation).
pub fn media_changer(
    config: &SectionConfigData,
    drive: &str,
) -> Result<Option<(Box<dyn MediaChange>, String)>, Error> {

    match config.sections.get(drive) {
        Some((section_type_name, config)) => {
            match section_type_name.as_ref() {
                "virtual" => {
                    let tape = VirtualTapeDrive::deserialize(config)?;
                    Ok(Some((Box::new(tape), drive.to_string())))
                }
                "linux" => {
                    let drive_config = LinuxTapeDrive::deserialize(config)?;
                    match drive_config.changer {
                        Some(ref changer_name) => {
                            let changer = MtxMediaChanger::with_drive_config(&drive_config)?;
                            let changer_name = changer_name.to_string();
                            Ok(Some((Box::new(changer), changer_name)))
                        }
                        None => Ok(None),
                    }
                }
                _ => bail!("unknown drive type '{}' - internal error"),
            }
        }
        None => {
            bail!("no such drive '{}'", drive);
        }
    }
}

/// Get the media changer (MediaChange + name) associated with a tape drive.
///
/// This fail if the drive has no associated changer device.
pub fn required_media_changer(
    config: &SectionConfigData,
    drive: &str,
) -> Result<(Box<dyn MediaChange>, String), Error> {
    match media_changer(config, drive) {
        Ok(Some(result)) => {
            return Ok(result);
        }
        Ok(None) => {
            bail!("drive '{}' has no associated changer device", drive);
        },
        Err(err) => {
            return Err(err);
        }
    }
}

pub fn open_drive(
    config: &SectionConfigData,
    drive: &str,
) -> Result<Box<dyn TapeDriver>, Error> {

    match config.sections.get(drive) {
        Some((section_type_name, config)) => {
            match section_type_name.as_ref() {
                "virtual" => {
                    let tape = VirtualTapeDrive::deserialize(config)?;
                    let handle = tape.open()?;
                    Ok(Box::new(handle))
                }
                "linux" => {
                    let tape = LinuxTapeDrive::deserialize(config)?;
                    let handle = tape.open()?;
                    Ok(Box::new(handle))
                }
                _ => bail!("unknown drive type '{}' - internal error"),
            }
        }
        None => {
            bail!("no such drive '{}'", drive);
        }
    }
}

/// Requests a specific 'media' to be inserted into 'drive'. Within a
/// loop, this then tries to read the media label and waits until it
/// finds the requested media.
///
/// Returns a handle to the opened drive and the media labels.
pub fn request_and_load_media(
    worker: &WorkerTask,
    config: &SectionConfigData,
    drive: &str,
    label: &MediaLabel,
) -> Result<(
    Box<dyn TapeDriver>,
    MediaId,
), Error> {

    let check_label = |handle: &mut dyn TapeDriver, uuid: &proxmox::tools::Uuid| {
        if let Ok(Some(media_id)) = handle.read_label() {
            worker.log(format!(
                "found media label {} ({})",
                media_id.label.label_text,
                media_id.label.uuid.to_string(),
            ));
            if media_id.label.uuid == *uuid {
                return Ok(media_id);
            }
        }
        bail!("read label failed (please label all tapes first)");
    };

    match config.sections.get(drive) {
        Some((section_type_name, config)) => {
            match section_type_name.as_ref() {
                "virtual" => {
                    let mut tape = VirtualTapeDrive::deserialize(config)?;

                    let label_text = label.label_text.clone();

                    tape.load_media(&label_text)?;

                    let mut handle: Box<dyn TapeDriver> = Box::new(tape.open()?);

                    let media_id = check_label(handle.as_mut(), &label.uuid)?;

                    return Ok((handle, media_id));
                }
                "linux" => {
                    let drive_config = LinuxTapeDrive::deserialize(config)?;

                    let label_text = label.label_text.clone();

                    if drive_config.changer.is_some() {

                        let mut changer = MtxMediaChanger::with_drive_config(&drive_config)?;
                        changer.load_media(&label_text)?;

                        let mut handle: Box<dyn TapeDriver> = Box::new(drive_config.open()?);

                        let media_id = check_label(handle.as_mut(), &label.uuid)?;

                        return Ok((handle, media_id));
                    }

                    worker.log(format!("Please insert media '{}' into drive '{}'", label_text, drive));

                    let to = "root@localhost"; // fixme

                    send_load_media_email(drive, &label_text, to)?;

                    let mut last_media_uuid = None;
                    let mut last_error = None;

                    loop {
                        let mut handle = match drive_config.open() {
                            Ok(handle) => handle,
                            Err(err) => {
                                let err = err.to_string();
                                if Some(err.clone()) != last_error {
                                    worker.log(format!("tape open failed - {}", err));
                                    last_error = Some(err);
                                }
                                std::thread::sleep(std::time::Duration::from_millis(5_000));
                                continue;
                            }
                        };

                        match handle.read_label() {
                            Ok(Some(media_id)) => {
                                if media_id.label.uuid == label.uuid {
                                    worker.log(format!(
                                        "found media label {} ({})",
                                        media_id.label.label_text,
                                        media_id.label.uuid.to_string(),
                                    ));
                                    return Ok((Box::new(handle), media_id));
                                } else {
                                    if Some(media_id.label.uuid.clone()) != last_media_uuid {
                                        worker.log(format!(
                                            "wrong media label {} ({})",
                                            media_id.label.label_text,
                                            media_id.label.uuid.to_string(),
                                        ));
                                        last_media_uuid = Some(media_id.label.uuid);
                                    }
                                }
                            }
                            Ok(None) => {
                                if last_media_uuid.is_some() {
                                    worker.log(format!("found empty media without label (please label all tapes first)"));
                                    last_media_uuid = None;
                                }
                            }
                            Err(err) => {
                                let err = err.to_string();
                                if Some(err.clone()) != last_error {
                                    worker.log(format!("tape open failed - {}", err));
                                    last_error = Some(err);
                                }
                            }
                        }

                        // eprintln!("read label failed -  test again in 5 secs");
                        std::thread::sleep(std::time::Duration::from_millis(5_000));
                    }
                }
                _ => bail!("drive type '{}' not implemented!"),
            }
        }
        None => {
            bail!("no such drive '{}'", drive);
        }
    }
}
