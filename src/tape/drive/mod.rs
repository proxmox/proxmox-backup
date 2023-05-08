//! Tape drivers

mod virtual_tape;

mod lto;
pub use lto::*;

use std::path::PathBuf;

use anyhow::{bail, format_err, Error};
use nix::fcntl::OFlag;
use nix::sys::stat::Mode;
use serde::Deserialize;
use serde_json::Value;

use proxmox_sys::fs::{
    atomic_open_or_create_file, file_read_optional_string, lock_file, replace_file, CreateOptions,
};

use proxmox_io::ReadExt;
use proxmox_section_config::SectionConfigData;
use proxmox_sys::{task_log, WorkerTaskContext};
use proxmox_uuid::Uuid;

use pbs_api_types::{Fingerprint, LtoTapeDrive, VirtualTapeDrive};
use pbs_key_config::KeyConfig;

use pbs_tape::{sg_tape::TapeAlertFlags, BlockReadError, MediaContentHeader, TapeRead, TapeWrite};

use crate::{
    server::send_load_media_email,
    tape::{
        changer::{MediaChange, MtxMediaChanger},
        drive::virtual_tape::open_virtual_tape_drive,
        file_formats::{
            MediaLabel, MediaSetLabel, PROXMOX_BACKUP_MEDIA_LABEL_MAGIC_1_0,
            PROXMOX_BACKUP_MEDIA_SET_LABEL_MAGIC_1_0,
        },
        MediaId,
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
    /// We assume this flushes the tape write buffer. if
    /// write_missing_eof is true, we verify that there is a filemark
    /// at the end. If not, we write one.
    fn move_to_eom(&mut self, write_missing_eof: bool) -> Result<(), Error>;

    /// Move to last file
    fn move_to_last_file(&mut self) -> Result<(), Error>;

    /// Move to given file nr
    fn move_to_file(&mut self, file: u64) -> Result<(), Error>;

    /// Current file number
    fn current_file_number(&mut self) -> Result<u64, Error>;

    /// Completely erase the media
    fn format_media(&mut self, fast: bool) -> Result<(), Error>;

    /// Read/Open the next file
    fn read_next_file<'a>(&'a mut self) -> Result<Box<dyn TapeRead + 'a>, BlockReadError>;

    /// Write/Append a new file
    fn write_file<'a>(&'a mut self) -> Result<Box<dyn TapeWrite + 'a>, std::io::Error>;

    /// Write label to tape (erase tape content)
    fn label_tape(&mut self, label: &MediaLabel) -> Result<(), Error> {
        self.set_encryption(None)?;

        self.format_media(true)?; // this rewinds the tape

        let raw = serde_json::to_string_pretty(&serde_json::to_value(label)?)?;

        let header =
            MediaContentHeader::new(PROXMOX_BACKUP_MEDIA_LABEL_MAGIC_1_0, raw.len() as u32);

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
            let mut reader = match self.read_next_file() {
                Err(BlockReadError::EndOfStream) => {
                    return Ok((None, None)); // tape is empty
                }
                Err(BlockReadError::EndOfFile) => {
                    bail!("got unexpected filemark at BOT");
                }
                Err(BlockReadError::Error(err)) => {
                    return Err(err.into());
                }
                Ok(reader) => reader,
            };

            let header: MediaContentHeader = unsafe { reader.read_le_value()? };
            header.check(PROXMOX_BACKUP_MEDIA_LABEL_MAGIC_1_0, 1, 64 * 1024)?;
            let data = reader.read_exact_allocated(header.size as usize)?;

            let label: MediaLabel = serde_json::from_slice(&data)
                .map_err(|err| format_err!("unable to parse drive label - {}", err))?;

            // make sure we read the EOF marker
            if reader.skip_to_end()? != 0 {
                bail!("got unexpected data after label");
            }

            label
        };

        let mut media_id = MediaId {
            label,
            media_set_label: None,
        };

        // try to read MediaSet label
        let mut reader = match self.read_next_file() {
            Err(BlockReadError::EndOfStream) => {
                return Ok((Some(media_id), None));
            }
            Err(BlockReadError::EndOfFile) => {
                bail!("got unexpected filemark after label");
            }
            Err(BlockReadError::Error(err)) => {
                return Err(err.into());
            }
            Ok(reader) => reader,
        };

        let header: MediaContentHeader = unsafe { reader.read_le_value()? };
        header.check(PROXMOX_BACKUP_MEDIA_SET_LABEL_MAGIC_1_0, 1, 64 * 1024)?;
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
    /// uuid (first 16 bytes), so that each media set uses an unique
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

/// A boxed implementor of [`MediaChange`].
pub type MediaChanger = Box<dyn MediaChange>;

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
) -> Result<Option<(MediaChanger, String)>, Error> {
    match config.sections.get(drive) {
        Some((section_type_name, config)) => match section_type_name.as_ref() {
            "virtual" => {
                let tape = VirtualTapeDrive::deserialize(config)?;
                Ok(Some((Box::new(tape), drive.to_string())))
            }
            "lto" => {
                let drive_config = LtoTapeDrive::deserialize(config)?;
                match drive_config.changer {
                    Some(ref changer_name) => {
                        let changer = MtxMediaChanger::with_drive_config(&drive_config)?;
                        let changer_name = changer_name.to_string();
                        Ok(Some((Box::new(changer), changer_name)))
                    }
                    None => Ok(None),
                }
            }
            ty => bail!("unknown drive type '{}' - internal error", ty),
        },
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
) -> Result<(MediaChanger, String), Error> {
    match media_changer(config, drive) {
        Ok(Some(result)) => Ok(result),
        Ok(None) => {
            bail!("drive '{}' has no associated changer device", drive);
        }
        Err(err) => Err(err),
    }
}

/// Opens a tape drive (this fails if there is no media loaded)
pub fn open_drive(config: &SectionConfigData, drive: &str) -> Result<Box<dyn TapeDriver>, Error> {
    match config.sections.get(drive) {
        Some((section_type_name, config)) => match section_type_name.as_ref() {
            "virtual" => {
                let tape = VirtualTapeDrive::deserialize(config)?;
                let handle = open_virtual_tape_drive(&tape)?;
                Ok(Box::new(handle))
            }
            "lto" => {
                let tape = LtoTapeDrive::deserialize(config)?;
                let handle = open_lto_tape_drive(&tape)?;
                Ok(Box::new(handle))
            }
            ty => bail!("unknown drive type '{}' - internal error", ty),
        },
        None => {
            bail!("no such drive '{}'", drive);
        }
    }
}

#[derive(PartialEq, Eq)]
enum TapeRequestError {
    None,
    EmptyTape,
    OpenFailed(String),
    WrongLabel(String),
    ReadFailed(String),
    LoadingFailed(String),
}

impl std::fmt::Display for TapeRequestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TapeRequestError::None => {
                write!(f, "no error")
            }
            TapeRequestError::OpenFailed(reason) => {
                write!(f, "tape open failed - {}", reason)
            }
            TapeRequestError::WrongLabel(label) => {
                write!(f, "wrong media label {}", label)
            }
            TapeRequestError::EmptyTape => {
                write!(
                    f,
                    "found empty media without label (please label all tapes first)"
                )
            }
            TapeRequestError::ReadFailed(reason) => {
                write!(f, "tape read failed - {}", reason)
            }
            TapeRequestError::LoadingFailed(reason) => {
                write!(f, "could not load tape into drive - {}", reason)
            }
        }
    }
}

/// Requests a specific 'media' to be inserted into 'drive'. Within a
/// loop, this then tries to read the media label and waits until it
/// finds the requested media.
///
/// Returns a handle to the opened drive and the media labels.
pub fn request_and_load_media(
    worker: &dyn WorkerTaskContext,
    config: &SectionConfigData,
    drive: &str,
    label: &MediaLabel,
    notify_email: &Option<String>,
) -> Result<(Box<dyn TapeDriver>, MediaId), Error> {
    let check_label = |handle: &mut dyn TapeDriver, uuid: &proxmox_uuid::Uuid| {
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

                    let mut handle: Box<dyn TapeDriver> = Box::new(open_virtual_tape_drive(&tape)?);

                    let media_id = check_label(handle.as_mut(), &label.uuid)?;

                    Ok((handle, media_id))
                }
                "lto" => {
                    let drive_config = LtoTapeDrive::deserialize(config)?;

                    let label_text = label.label_text.clone();

                    let mut last_error = TapeRequestError::None;

                    let changer = &drive_config.changer;

                    let update_and_log_request_error =
                        |old: &mut TapeRequestError, new: TapeRequestError| -> Result<(), Error> {
                            if new != *old {
                                task_log!(worker, "{}", new);
                                let (device_type, device) = if let Some(changer) = changer {
                                    ("changer", changer.as_str())
                                } else {
                                    ("drive", drive)
                                };

                                task_log!(
                                    worker,
                                    "Please insert media '{}' into {} '{}'",
                                    label_text,
                                    device_type,
                                    device
                                );
                                if let Some(to) = notify_email {
                                    send_load_media_email(
                                        changer.is_some(),
                                        device,
                                        &label_text,
                                        to,
                                        Some(new.to_string()),
                                    )?;
                                }
                                *old = new;
                            }
                            Ok(())
                        };

                    loop {
                        worker.check_abort()?;

                        if last_error != TapeRequestError::None {
                            for _ in 0..50 {
                                // delay 5 seconds
                                worker.check_abort()?;
                                std::thread::sleep(std::time::Duration::from_millis(100));
                            }
                        } else if drive_config.changer.is_none() {
                            task_log!(
                                worker,
                                "Checking for media '{}' in drive '{}'",
                                label_text,
                                drive
                            );
                        } else {
                            task_log!(
                                worker,
                                "trying to load media '{}' into drive '{}'",
                                label_text,
                                drive
                            );
                        }

                        if drive_config.changer.is_some() {
                            let mut changer = MtxMediaChanger::with_drive_config(&drive_config)?;
                            if let Err(err) = changer.load_media(&label_text) {
                                update_and_log_request_error(
                                    &mut last_error,
                                    TapeRequestError::LoadingFailed(err.to_string()),
                                )?;
                                continue;
                            }
                        }

                        let mut handle = match open_lto_tape_drive(&drive_config) {
                            Ok(handle) => handle,
                            Err(err) => {
                                update_and_log_request_error(
                                    &mut last_error,
                                    TapeRequestError::OpenFailed(err.to_string()),
                                )?;
                                continue;
                            }
                        };

                        let request_error = match handle.read_label() {
                            Ok((Some(media_id), _)) if media_id.label.uuid == label.uuid => {
                                task_log!(
                                    worker,
                                    "found media label {} ({})",
                                    media_id.label.label_text,
                                    media_id.label.uuid.to_string(),
                                );
                                return Ok((Box::new(handle), media_id));
                            }
                            Ok((Some(media_id), _)) => {
                                let label_string = format!(
                                    "{} ({})",
                                    media_id.label.label_text, media_id.label.uuid,
                                );
                                TapeRequestError::WrongLabel(label_string)
                            }
                            Ok((None, _)) => TapeRequestError::EmptyTape,
                            Err(err) => TapeRequestError::ReadFailed(err.to_string()),
                        };

                        update_and_log_request_error(&mut last_error, request_error)?;
                    }
                }
                ty => bail!("drive type '{}' not implemented!", ty),
            }
        }
        None => {
            bail!("no such drive '{}'", drive);
        }
    }
}

#[derive(thiserror::Error, Debug)]
pub enum TapeLockError {
    #[error("timeout while trying to lock")]
    TimeOut,
    #[error("{0}")]
    Other(#[from] Error),
}

impl From<std::io::Error> for TapeLockError {
    fn from(error: std::io::Error) -> Self {
        Self::Other(error.into())
    }
}

/// Acquires an exclusive lock for the tape device
///
/// Basically calls lock_device_path() using the configured drive path.
pub fn lock_tape_device(
    config: &SectionConfigData,
    drive: &str,
) -> Result<DeviceLockGuard, TapeLockError> {
    let path = tape_device_path(config, drive)?;
    lock_device_path(&path).map_err(|err| match err {
        TapeLockError::Other(err) => {
            TapeLockError::Other(format_err!("unable to lock drive '{}' - {}", drive, err))
        }
        other => other,
    })
}

/// Writes the given state for the specified drive
///
/// This function does not lock, so make sure the drive is locked
pub fn set_tape_device_state(drive: &str, state: &str) -> Result<(), Error> {
    let mut path = PathBuf::from(crate::tape::DRIVE_STATE_DIR);
    path.push(drive);

    let backup_user = pbs_config::backup_user()?;
    let mode = nix::sys::stat::Mode::from_bits_truncate(0o0644);
    let options = CreateOptions::new()
        .perm(mode)
        .owner(backup_user.uid)
        .group(backup_user.gid);

    replace_file(path, state.as_bytes(), options, false)
}

/// Get the device state
pub fn get_tape_device_state(
    config: &SectionConfigData,
    drive: &str,
) -> Result<Option<String>, Error> {
    let path = format!("{}/{}", crate::tape::DRIVE_STATE_DIR, drive);
    let state = file_read_optional_string(path)?;

    let device_path = tape_device_path(config, drive)?;
    if test_device_path_lock(&device_path)? {
        Ok(state)
    } else {
        Ok(None)
    }
}

fn tape_device_path(config: &SectionConfigData, drive: &str) -> Result<String, Error> {
    match config.sections.get(drive) {
        Some((section_type_name, config)) => {
            let path = match section_type_name.as_ref() {
                "virtual" => VirtualTapeDrive::deserialize(config)?.path,
                "lto" => LtoTapeDrive::deserialize(config)?.path,
                ty => bail!("unknown drive type '{}' - internal error", ty),
            };
            Ok(path)
        }
        None => {
            bail!("no such drive '{}'", drive);
        }
    }
}

pub struct DeviceLockGuard(std::fs::File);

// Uses systemd escape_unit to compute a file name from `device_path`, the try
// to lock `/var/lock/<name>`.
fn open_device_lock(device_path: &str) -> Result<std::fs::File, Error> {
    let lock_name = proxmox_sys::systemd::escape_unit(device_path, true);

    let mut path = std::path::PathBuf::from(crate::tape::DRIVE_LOCK_DIR);
    path.push(lock_name);

    let user = pbs_config::backup_user()?;
    let options = CreateOptions::new()
        .perm(Mode::from_bits_truncate(0o660))
        .owner(user.uid)
        .group(user.gid);

    atomic_open_or_create_file(
        path,
        OFlag::O_RDWR | OFlag::O_CLOEXEC | OFlag::O_APPEND,
        &[],
        options,
        false,
    )
}

// Acquires an exclusive lock on `device_path`
//
fn lock_device_path(device_path: &str) -> Result<DeviceLockGuard, TapeLockError> {
    let mut file = open_device_lock(device_path)?;
    let timeout = std::time::Duration::new(10, 0);
    if let Err(err) = lock_file(&mut file, true, Some(timeout)) {
        if err.kind() == std::io::ErrorKind::Interrupted {
            return Err(TapeLockError::TimeOut);
        } else {
            return Err(err.into());
        }
    }

    Ok(DeviceLockGuard(file))
}

// Same logic as lock_device_path, but uses a timeout of 0, making it
// non-blocking, and returning if the file is locked or not
fn test_device_path_lock(device_path: &str) -> Result<bool, Error> {
    let mut file = open_device_lock(device_path)?;

    let timeout = std::time::Duration::new(0, 0);
    match lock_file(&mut file, true, Some(timeout)) {
        // file was not locked, continue
        Ok(()) => {}
        // file was locked, return true
        Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => return Ok(true),
        Err(err) => bail!("{}", err),
    }

    Ok(false)
}
