//! Tape drivers

mod virtual_tape;

pub mod linux_mtio;

mod tape_alert_flags;
pub use tape_alert_flags::*;

mod volume_statistics;
pub use volume_statistics::*;

mod encryption;
pub use encryption::*;

mod linux_tape;
pub use linux_tape::*;

mod mam;
pub use mam::*;

use std::os::unix::io::AsRawFd;

use anyhow::{bail, format_err, Error};
use ::serde::{Deserialize};
use serde_json::Value;

use proxmox::{
    tools::{
        Uuid,
        io::ReadExt,
        fs::fchown,
    },
    api::section_config::SectionConfigData,
};

use crate::{
    task_log,
    task::TaskState,
    backup::{
        Fingerprint,
        KeyConfig,
    },
    api2::types::{
        VirtualTapeDrive,
        LinuxTapeDrive,
    },
    server::WorkerTask,
    tape::{
        TapeWrite,
        TapeRead,
        MediaId,
        file_formats::{
            PROXMOX_BACKUP_MEDIA_LABEL_MAGIC_1_0,
            PROXMOX_BACKUP_MEDIA_SET_LABEL_MAGIC_1_0,
            MediaLabel,
            MediaSetLabel,
            MediaContentHeader,
        },
        changer::{
            MediaChange,
            MtxMediaChanger,
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

        self.set_encryption(None)?;

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
    ///
    /// If the media-set is encrypted, we also store the encryption
    /// key_config, so that it is possible to restore the key.
    fn write_media_set_label(
        &mut self,
        media_set_label: &MediaSetLabel,
        key_config: Option<&KeyConfig>,
    ) -> Result<(), Error>;

    /// Read the media label
    ///
    /// This tries to read both media labels (label and
    /// media_set_label). Also returns the optional encryption key configuration.
    fn read_label(&mut self) -> Result<(Option<MediaId>, Option<KeyConfig>), Error> {

        self.rewind()?;

        let label = {
            let mut reader = match self.read_next_file()? {
                None => return Ok((None, None)), // tape is empty
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
            None => return Ok((Some(media_id), None)),
            Some(reader) => reader,
        };

        let header: MediaContentHeader = unsafe { reader.read_le_value()? };
        header.check(PROXMOX_BACKUP_MEDIA_SET_LABEL_MAGIC_1_0, 1, 64*1024)?;
        let data = reader.read_exact_allocated(header.size as usize)?;

        let mut data: Value = serde_json::from_slice(&data)
            .map_err(|err| format_err!("unable to parse media set label - {}", err))?;

        let key_config_value = data["key-config"].take();
        let key_config: Option<KeyConfig> = if !key_config_value.is_null() {
            Some(serde_json::from_value(key_config_value)?)
        } else {
            None
        };

        let media_set_label: MediaSetLabel = serde_json::from_value(data)
            .map_err(|err| format_err!("unable to parse media set label - {}", err))?;

        // make sure we read the EOF marker
        if reader.skip_to_end()? != 0 {
            bail!("got unexpected data after media set label");
        }

        media_id.media_set_label = Some(media_set_label);

        Ok((Some(media_id), key_config))
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
    ///
    /// We use the media_set_uuid to XOR the secret key with the
    /// uuid (first 16 bytes), so that each media set uses an uique
    /// key for encryption.
    fn set_encryption(
        &mut self,
        key_fingerprint: Option<(Fingerprint, Uuid)>,
    ) -> Result<(), Error> {
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
            Ok(result)
        }
        Ok(None) => {
            bail!("drive '{}' has no associated changer device", drive);
        },
        Err(err) => {
            Err(err)
        }
    }
}

/// Opens a tape drive (this fails if there is no media loaded)
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
        if let Ok((Some(media_id), _)) = handle.read_label() {
            task_log!(
                worker,
                "found media label {} ({})",
                media_id.label.label_text,
                media_id.label.uuid,
            );

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

                    Ok((handle, media_id))
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

                    task_log!(worker, "Please insert media '{}' into drive '{}'", label_text, drive);

                    let to = "root@localhost"; // fixme

                    send_load_media_email(drive, &label_text, to)?;

                    let mut last_media_uuid = None;
                    let mut last_error = None;

                    loop {
                        worker.check_abort()?;

                        let mut handle = match drive_config.open() {
                            Ok(handle) => handle,
                            Err(err) => {
                                let err = err.to_string();
                                if Some(err.clone()) != last_error {
                                    task_log!(worker, "tape open failed - {}", err);
                                    last_error = Some(err);
                                }
                                for _ in 0..50 { // delay 5 seconds
                                    worker.check_abort()?;
                                    std::thread::sleep(std::time::Duration::from_millis(100));
                                }
                                continue;
                            }
                        };

                        match handle.read_label() {
                            Ok((Some(media_id), _)) => {
                                if media_id.label.uuid == label.uuid {
                                    task_log!(
                                        worker,
                                        "found media label {} ({})",
                                        media_id.label.label_text,
                                        media_id.label.uuid.to_string(),
                                    );
                                    return Ok((Box::new(handle), media_id));
                                } else if Some(media_id.label.uuid.clone()) != last_media_uuid {
                                    task_log!(
                                        worker,
                                        "wrong media label {} ({})",
                                        media_id.label.label_text,
                                        media_id.label.uuid.to_string(),
                                    );
                                    last_media_uuid = Some(media_id.label.uuid);
                                }
                            }
                            Ok((None, _)) => {
                                if last_media_uuid.is_some() {
                                    task_log!(worker, "found empty media without label (please label all tapes first)");
                                    last_media_uuid = None;
                                }
                            }
                            Err(err) => {
                                let err = err.to_string();
                                if Some(err.clone()) != last_error {
                                    task_log!(worker, "tape open failed - {}", err);
                                    last_error = Some(err);
                                }
                            }
                        }

                        // eprintln!("read label failed -  test again in 5 secs");
                        for _ in 0..50 { // delay 5 seconds
                            worker.check_abort()?;
                            std::thread::sleep(std::time::Duration::from_millis(100));
                        }
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

/// Aquires an exclusive lock for the tape device
///
/// Basically calls lock_device_path() using the configured drive path.
pub fn lock_tape_device(
    config: &SectionConfigData,
    drive: &str,
) -> Result<DeviceLockGuard, Error> {

    match config.sections.get(drive) {
        Some((section_type_name, config)) => {
            let path = match section_type_name.as_ref() {
                "virtual" => {
                    VirtualTapeDrive::deserialize(config)?.path
                }
                "linux" => {
                    LinuxTapeDrive::deserialize(config)?.path
                }
                _ => bail!("unknown drive type '{}' - internal error"),
            };
            lock_device_path(&path)
                .map_err(|err| format_err!("unable to lock drive '{}' - {}", drive, err))
        }
        None => {
            bail!("no such drive '{}'", drive);
        }
    }
}

pub struct DeviceLockGuard(std::fs::File);

// Aquires an exclusive lock on `device_path`
//
// Uses systemd escape_unit to compute a file name from `device_path`, the try
// to lock `/var/lock/<name>`.
fn lock_device_path(device_path: &str) -> Result<DeviceLockGuard, Error> {

    let lock_name = crate::tools::systemd::escape_unit(device_path, true);

    let mut path = std::path::PathBuf::from("/var/lock");
    path.push(lock_name);

    let timeout = std::time::Duration::new(10, 0);
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
    proxmox::tools::fs::lock_file(&mut file, true, Some(timeout))?;

    let backup_user = crate::backup::backup_user()?;
    fchown(file.as_raw_fd(), Some(backup_user.uid), Some(backup_user.gid))?;

    Ok(DeviceLockGuard(file))
}

// Same logic as lock_device_path, but uses a timeout of 0, making it
// non-blocking, and returning if the file is locked or not
fn test_device_path_lock(device_path: &str) -> Result<bool, Error> {

    let lock_name = crate::tools::systemd::escape_unit(device_path, true);

    let mut path = std::path::PathBuf::from("/var/lock");
    path.push(lock_name);

    let timeout = std::time::Duration::new(0, 0);
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
    match proxmox::tools::fs::lock_file(&mut file, true, Some(timeout)) {
        // file was not locked, continue
        Ok(()) => {},
        // file was locked, return true
        Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => return Ok(true),
        Err(err) => bail!("{}", err),
    }

    let backup_user = crate::backup::backup_user()?;
    fchown(file.as_raw_fd(), Some(backup_user.uid), Some(backup_user.gid))?;

    Ok(false)
}
