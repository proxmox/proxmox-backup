mod virtual_tape;
mod linux_mtio;
mod linux_tape;

mod linux_list_drives;
pub use linux_list_drives::*;

use anyhow::{bail, format_err, Error};
use ::serde::{Deserialize, Serialize};

use proxmox::tools::Uuid;
use proxmox::tools::io::ReadExt;
use proxmox::api::section_config::SectionConfigData;

use crate::{
    api2::types::{
        VirtualTapeDrive,
        LinuxTapeDrive,
    },
    tape::{
        TapeWrite,
        TapeRead,
        file_formats::{
            PROXMOX_BACKUP_MEDIA_LABEL_MAGIC_1_0,
            PROXMOX_BACKUP_MEDIA_SET_LABEL_MAGIC_1_0,
            MediaLabel,
            MediaSetLabel,
            MediaContentHeader,
        },
        changer::{
            MediaChange,
            ChangeMediaEmail,
        },
    },
};

#[derive(Serialize,Deserialize)]
pub struct MediaLabelInfo {
    pub label: MediaLabel,
    pub label_uuid: Uuid,
    #[serde(skip_serializing_if="Option::is_none")]
    pub media_set_label: Option<(MediaSetLabel, Uuid)>
}

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
    fn current_file_number(&mut self) -> Result<usize, Error>;

    /// Completely erase the media
    fn erase_media(&mut self, fast: bool) -> Result<(), Error>;

    /// Read/Open the next file
    fn read_next_file<'a>(&'a mut self) -> Result<Option<Box<dyn TapeRead + 'a>>, std::io::Error>;

    /// Write/Append a new file
    fn write_file<'a>(&'a mut self) -> Result<Box<dyn TapeWrite + 'a>, std::io::Error>;

    /// Write label to tape (erase tape content)
    ///
    /// This returns the MediaContentHeader uuid (not the media uuid).
    fn label_tape(&mut self, label: &MediaLabel) -> Result<Uuid, Error> {

        self.rewind()?;

        self.erase_media(true)?;

        let raw = serde_json::to_string_pretty(&serde_json::to_value(&label)?)?;

        let header = MediaContentHeader::new(PROXMOX_BACKUP_MEDIA_LABEL_MAGIC_1_0, raw.len() as u32);
        let content_uuid = header.content_uuid();

        {
            let mut writer = self.write_file()?;
            writer.write_header(&header, raw.as_bytes())?;
            writer.finish(false)?;
        }

        self.sync()?; // sync data to tape

        Ok(content_uuid)
    }

    /// Write the media set label to tape
    ///
    /// This returns the MediaContentHeader uuid (not the media uuid).
    fn write_media_set_label(&mut self, media_set_label: &MediaSetLabel) -> Result<Uuid, Error>;

    /// Read the media label
    ///
    /// This tries to read both media labels (label and media_set_label).
    fn read_label(&mut self) -> Result<Option<MediaLabelInfo>, Error> {

        self.rewind()?;

        let (label, label_uuid) = {
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

            (label, Uuid::from(header.uuid))
        };

        let mut info = MediaLabelInfo { label, label_uuid, media_set_label: None };

         // try to read MediaSet label
        let mut reader = match self.read_next_file()? {
            None => return Ok(Some(info)),
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

        info.media_set_label = Some((media_set_label, Uuid::from(header.uuid)));

        Ok(Some(info))
    }

    /// Eject media
    fn eject_media(&mut self) -> Result<(), Error>;
}

/// Get the media changer (name + MediaChange) associated with a tape drie.
///
/// If allow_email is set, returns an ChangeMediaEmail instance for
/// standalone tape drives (changer name set to "").
pub fn media_changer(
    config: &SectionConfigData,
    drive: &str,
    allow_email: bool,
) -> Result<(Box<dyn MediaChange>, String), Error> {

    match config.sections.get(drive) {
        Some((section_type_name, config)) => {
            match section_type_name.as_ref() {
                "virtual" => {
                    let tape = VirtualTapeDrive::deserialize(config)?;
                    Ok((Box::new(tape), drive.to_string()))
                }
                "linux" => {
                    let tape = LinuxTapeDrive::deserialize(config)?;
                    match tape.changer {
                        Some(ref changer_name) => {
                            let changer_name = changer_name.to_string();
                            Ok((Box::new(tape), changer_name))
                        }
                        None =>  {
                            if !allow_email {
                                bail!("drive '{}' has no changer device", drive);
                            }
                            let to = "root@localhost"; // fixme
                            let changer = ChangeMediaEmail::new(drive, to);
                            Ok((Box::new(changer), String::new()))
                        },
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

pub fn open_drive(
    config: &SectionConfigData,
    drive: &str,
) -> Result<Box<dyn TapeDriver>, Error> {

    match config.sections.get(drive) {
        Some((section_type_name, config)) => {
            match section_type_name.as_ref() {
                "virtual" => {
                    let tape = VirtualTapeDrive::deserialize(config)?;
                    let handle = tape.open()
                        .map_err(|err| format_err!("open drive '{}' ({}) failed - {}", drive, tape.path, err))?;
                   Ok(Box::new(handle))
                }
                "linux" => {
                    let tape = LinuxTapeDrive::deserialize(config)?;
                    let handle = tape.open()
                        .map_err(|err| format_err!("open drive '{}' ({}) failed - {}", drive, tape.path, err))?;
                    Ok(Box::new(handle))
                }
                _ => bail!("drive type '{}' not implemented!"),
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
    config: &SectionConfigData,
    drive: &str,
    label: &MediaLabel,
) -> Result<(
    Box<dyn TapeDriver>,
    MediaLabelInfo,
), Error> {

    match config.sections.get(drive) {
        Some((section_type_name, config)) => {
            match section_type_name.as_ref() {
                "virtual" => {
                    let mut drive = VirtualTapeDrive::deserialize(config)?;

                    let changer_id = label.changer_id.clone();

                    drive.load_media(&changer_id)?;

                    let mut handle = drive.open()?;

                    if let Ok(Some(info)) = handle.read_label() {
                        println!("found media label {} ({})", info.label.changer_id, info.label.uuid.to_string());
                        if info.label.uuid == label.uuid {
                            return Ok((Box::new(handle), info));
                        }
                    }
                    bail!("read label failed (label all tapes first)");
                }
                "linux" => {
                    let tape = LinuxTapeDrive::deserialize(config)?;

                    let id = label.changer_id.clone();

                    println!("Please insert media '{}' into drive '{}'", id, drive);

                    loop {
                        let mut handle = match tape.open() {
                            Ok(handle) => handle,
                            Err(_) => {
                                eprintln!("tape open failed - test again in 5 secs");
                                std::thread::sleep(std::time::Duration::from_millis(5_000));
                                continue;
                            }
                        };

                        if let Ok(Some(info)) = handle.read_label() {
                            println!("found media label {} ({})", info.label.changer_id, info.label.uuid.to_string());
                            if info.label.uuid == label.uuid {
                                return Ok((Box::new(handle), info));
                            }
                        }

                        println!("read label failed -  test again in 5 secs");
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
