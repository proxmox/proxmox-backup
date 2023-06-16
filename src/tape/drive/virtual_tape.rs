// Note: This is only for test an debug

use std::fs::File;
use std::io;

use anyhow::{bail, format_err, Error};
use serde::{Deserialize, Serialize};

use proxmox_sys::fs::{replace_file, CreateOptions};

use pbs_key_config::KeyConfig;
use pbs_tape::{
    BlockReadError, BlockedReader, BlockedWriter, DriveStatus, ElementStatus, EmulateTapeReader,
    EmulateTapeWriter, MediaContentHeader, MtxStatus, StorageElementStatus, TapeRead, TapeWrite,
};

use crate::tape::{
    drive::{MediaChange, TapeDriver, VirtualTapeDrive},
    file_formats::{MediaSetLabel, PROXMOX_BACKUP_MEDIA_SET_LABEL_MAGIC_1_0},
};

/// This needs to lock the drive
pub fn open_virtual_tape_drive(config: &VirtualTapeDrive) -> Result<VirtualTapeHandle, Error> {
    proxmox_lang::try_block!({
        let mut lock_path = std::path::PathBuf::from(&config.path);
        lock_path.push(".drive.lck");

        let options = CreateOptions::new();
        let timeout = std::time::Duration::new(10, 0);
        let lock = proxmox_sys::fs::open_file_locked(&lock_path, timeout, true, options)?;

        Ok(VirtualTapeHandle {
            _lock: lock,
            drive_name: config.name.clone(),
            max_size: config.max_size.unwrap_or(64 * 1024 * 1024),
            path: std::path::PathBuf::from(&config.path),
        })
    })
    .map_err(|err: Error| {
        format_err!(
            "open drive '{}' ({}) failed - {}",
            config.name,
            config.path,
            err
        )
    })
}

#[derive(Serialize, Deserialize)]
struct VirtualTapeStatus {
    name: String,
    pos: usize,
}

#[derive(Serialize, Deserialize)]
struct VirtualDriveStatus {
    current_tape: Option<VirtualTapeStatus>,
}

#[derive(Serialize, Deserialize)]
struct TapeIndex {
    files: usize,
}

pub struct VirtualTapeHandle {
    drive_name: String,
    path: std::path::PathBuf,
    max_size: usize,
    _lock: File,
}

impl VirtualTapeHandle {
    fn status_file_path(&self) -> std::path::PathBuf {
        let mut path = self.path.clone();
        path.push("drive-status.json");
        path
    }

    fn tape_index_path(&self, tape_name: &str) -> std::path::PathBuf {
        let mut path = self.path.clone();
        path.push(format!("tape-{}.json", tape_name));
        path
    }

    fn tape_file_path(&self, tape_name: &str, pos: usize) -> std::path::PathBuf {
        let mut path = self.path.clone();
        path.push(format!("tapefile-{}-{}.json", pos, tape_name));
        path
    }

    fn load_tape_index(&self, tape_name: &str) -> Result<TapeIndex, Error> {
        let path = self.tape_index_path(tape_name);
        let raw = proxmox_sys::fs::file_get_contents(path)?;
        if raw.is_empty() {
            return Ok(TapeIndex { files: 0 });
        }
        let data: TapeIndex = serde_json::from_slice(&raw)?;
        Ok(data)
    }

    fn store_tape_index(&self, tape_name: &str, index: &TapeIndex) -> Result<(), Error> {
        let path = self.tape_index_path(tape_name);
        let raw = serde_json::to_string_pretty(&serde_json::to_value(index)?)?;

        let options = CreateOptions::new();
        replace_file(path, raw.as_bytes(), options, false)?;
        Ok(())
    }

    fn truncate_tape(&self, tape_name: &str, pos: usize) -> Result<usize, Error> {
        let mut index = self.load_tape_index(tape_name)?;

        if index.files <= pos {
            return Ok(index.files);
        }

        for i in pos..index.files {
            let path = self.tape_file_path(tape_name, i);
            let _ = std::fs::remove_file(path);
        }

        index.files = pos;

        self.store_tape_index(tape_name, &index)?;

        Ok(index.files)
    }

    fn load_status(&self) -> Result<VirtualDriveStatus, Error> {
        let path = self.status_file_path();

        let default = serde_json::to_value(VirtualDriveStatus { current_tape: None })?;

        let data = proxmox_sys::fs::file_get_json(path, Some(default))?;
        let status: VirtualDriveStatus = serde_json::from_value(data)?;
        Ok(status)
    }

    fn store_status(&self, status: &VirtualDriveStatus) -> Result<(), Error> {
        let path = self.status_file_path();
        let raw = serde_json::to_string_pretty(&serde_json::to_value(status)?)?;

        let options = CreateOptions::new();
        replace_file(path, raw.as_bytes(), options, false)?;
        Ok(())
    }

    fn online_media_label_texts(&self) -> Result<Vec<String>, Error> {
        let mut list = Vec::new();
        for entry in std::fs::read_dir(&self.path)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_file() && path.extension() == Some(std::ffi::OsStr::new("json")) {
                if let Some(name) = path.file_stem() {
                    if let Some(name) = name.to_str() {
                        if let Some(label) = name.strip_prefix("tape-") {
                            list.push(label.to_string());
                        }
                    }
                }
            }
        }
        Ok(list)
    }

    #[allow(dead_code)]
    fn forward_space_count_files(&mut self, count: usize) -> Result<(), Error> {
        let mut status = self.load_status()?;
        match status.current_tape {
            Some(VirtualTapeStatus {
                ref name,
                ref mut pos,
            }) => {
                let index = self
                    .load_tape_index(name)
                    .map_err(|err| io::Error::new(io::ErrorKind::Other, err.to_string()))?;

                let new_pos = *pos + count;
                if new_pos <= index.files {
                    *pos = new_pos;
                } else {
                    bail!("forward_space_count_files failed: move beyond EOT");
                }

                self.store_status(&status)
                    .map_err(|err| io::Error::new(io::ErrorKind::Other, err.to_string()))?;

                Ok(())
            }
            None => bail!("drive is empty (no tape loaded)."),
        }
    }

    // Note: behavior differs from LTO, because we always position at
    // EOT side.
    fn backward_space_count_files(&mut self, count: usize) -> Result<(), Error> {
        let mut status = self.load_status()?;
        match status.current_tape {
            Some(VirtualTapeStatus { ref mut pos, .. }) => {
                if count <= *pos {
                    *pos -= count;
                } else {
                    bail!("backward_space_count_files failed: move before BOT");
                }

                self.store_status(&status)
                    .map_err(|err| io::Error::new(io::ErrorKind::Other, err.to_string()))?;

                Ok(())
            }
            None => bail!("drive is empty (no tape loaded)."),
        }
    }
}

impl TapeDriver for VirtualTapeHandle {
    fn sync(&mut self) -> Result<(), Error> {
        Ok(()) // do nothing for now
    }

    fn current_file_number(&mut self) -> Result<u64, Error> {
        let status = self
            .load_status()
            .map_err(|err| format_err!("current_file_number failed: {}", err.to_string()))?;

        match status.current_tape {
            Some(VirtualTapeStatus { pos, .. }) => Ok(pos as u64),
            None => bail!("current_file_number failed: drive is empty (no tape loaded)."),
        }
    }

    /// Move to last file
    fn move_to_last_file(&mut self) -> Result<(), Error> {
        self.move_to_eom(false)?;

        if self.current_file_number()? == 0 {
            bail!("move_to_last_file failed - media contains no data");
        }

        self.backward_space_count_files(1)?;

        Ok(())
    }

    fn move_to_file(&mut self, file: u64) -> Result<(), Error> {
        let mut status = self.load_status()?;
        match status.current_tape {
            Some(VirtualTapeStatus {
                ref name,
                ref mut pos,
            }) => {
                let index = self
                    .load_tape_index(name)
                    .map_err(|err| io::Error::new(io::ErrorKind::Other, err.to_string()))?;

                if file as usize > index.files {
                    bail!("invalid file nr");
                }

                *pos = file as usize;

                self.store_status(&status)
                    .map_err(|err| io::Error::new(io::ErrorKind::Other, err.to_string()))?;

                Ok(())
            }
            None => bail!("drive is empty (no tape loaded)."),
        }
    }

    fn read_next_file(&mut self) -> Result<Box<dyn TapeRead>, BlockReadError> {
        let mut status = self.load_status().map_err(|err| {
            BlockReadError::Error(io::Error::new(io::ErrorKind::Other, err.to_string()))
        })?;

        match status.current_tape {
            Some(VirtualTapeStatus {
                ref name,
                ref mut pos,
            }) => {
                let index = self.load_tape_index(name).map_err(|err| {
                    BlockReadError::Error(io::Error::new(io::ErrorKind::Other, err.to_string()))
                })?;

                if *pos >= index.files {
                    return Err(BlockReadError::EndOfStream);
                }

                let path = self.tape_file_path(name, *pos);
                let file = std::fs::OpenOptions::new().read(true).open(path)?;

                *pos += 1;
                self.store_status(&status).map_err(|err| {
                    BlockReadError::Error(io::Error::new(io::ErrorKind::Other, err.to_string()))
                })?;

                let reader = EmulateTapeReader::new(file);
                let reader = BlockedReader::open(reader)?;
                Ok(Box::new(reader))
            }
            None => Err(BlockReadError::Error(proxmox_lang::io_format_err!(
                "drive is empty (no tape loaded)."
            ))),
        }
    }

    fn write_file(&mut self) -> Result<Box<dyn TapeWrite>, io::Error> {
        let mut status = self
            .load_status()
            .map_err(|err| io::Error::new(io::ErrorKind::Other, err.to_string()))?;

        match status.current_tape {
            Some(VirtualTapeStatus {
                ref name,
                ref mut pos,
            }) => {
                let mut index = self
                    .load_tape_index(name)
                    .map_err(|err| io::Error::new(io::ErrorKind::Other, err.to_string()))?;

                for i in *pos..index.files {
                    let path = self.tape_file_path(name, i);
                    let _ = std::fs::remove_file(path);
                }

                let mut used_space = 0;
                for i in 0..*pos {
                    let path = self.tape_file_path(name, i);
                    used_space += path.metadata()?.len() as usize;
                }
                index.files = *pos + 1;

                self.store_tape_index(name, &index)
                    .map_err(|err| io::Error::new(io::ErrorKind::Other, err.to_string()))?;

                let path = self.tape_file_path(name, *pos);
                let file = std::fs::OpenOptions::new()
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .open(path)?;

                *pos = index.files;

                self.store_status(&status)
                    .map_err(|err| io::Error::new(io::ErrorKind::Other, err.to_string()))?;

                let mut free_space = 0;
                if used_space < self.max_size {
                    free_space = self.max_size - used_space;
                }

                let writer = EmulateTapeWriter::new(file, free_space);
                let writer = Box::new(BlockedWriter::new(writer));

                Ok(writer)
            }
            None => proxmox_lang::io_bail!("drive is empty (no tape loaded)."),
        }
    }

    fn move_to_eom(&mut self, _write_missing_eof: bool) -> Result<(), Error> {
        let mut status = self.load_status()?;
        match status.current_tape {
            Some(VirtualTapeStatus {
                ref name,
                ref mut pos,
            }) => {
                let index = self
                    .load_tape_index(name)
                    .map_err(|err| io::Error::new(io::ErrorKind::Other, err.to_string()))?;

                *pos = index.files;

                self.store_status(&status)
                    .map_err(|err| io::Error::new(io::ErrorKind::Other, err.to_string()))?;

                Ok(())
            }
            None => bail!("drive is empty (no tape loaded)."),
        }
    }

    fn rewind(&mut self) -> Result<(), Error> {
        let mut status = self.load_status()?;
        match status.current_tape {
            Some(ref mut tape_status) => {
                tape_status.pos = 0;
                self.store_status(&status)?;
                Ok(())
            }
            None => bail!("drive is empty (no tape loaded)."),
        }
    }

    fn format_media(&mut self, _fast: bool) -> Result<(), Error> {
        let mut status = self.load_status()?;
        match status.current_tape {
            Some(VirtualTapeStatus {
                ref name,
                ref mut pos,
            }) => {
                *pos = self.truncate_tape(name, 0)?;
                self.store_status(&status)?;
                Ok(())
            }
            None => bail!("drive is empty (no tape loaded)."),
        }
    }

    fn write_media_set_label(
        &mut self,
        media_set_label: &MediaSetLabel,
        key_config: Option<&KeyConfig>,
    ) -> Result<(), Error> {
        self.set_encryption(None)?;

        if key_config.is_some() {
            bail!("encryption is not implemented - internal error");
        }

        let mut status = self.load_status()?;
        match status.current_tape {
            Some(VirtualTapeStatus {
                ref name,
                ref mut pos,
            }) => {
                *pos = self.truncate_tape(name, 1)?;
                let pos = *pos;
                self.store_status(&status)?;

                if pos == 0 {
                    bail!("media is empty (no label).");
                }
                if pos != 1 {
                    bail!(
                        "write_media_set_label: truncate failed - got wrong pos '{}'",
                        pos
                    );
                }

                let raw = serde_json::to_string_pretty(&serde_json::to_value(media_set_label)?)?;
                let header = MediaContentHeader::new(
                    PROXMOX_BACKUP_MEDIA_SET_LABEL_MAGIC_1_0,
                    raw.len() as u32,
                );

                {
                    let mut writer = self.write_file()?;
                    writer.write_header(&header, raw.as_bytes())?;
                    writer.finish(false)?;
                }

                Ok(())
            }
            None => bail!("drive is empty (no tape loaded)."),
        }
    }

    fn eject_media(&mut self) -> Result<(), Error> {
        let status = VirtualDriveStatus { current_tape: None };
        self.store_status(&status)
    }
}

impl MediaChange for VirtualTapeHandle {
    fn drive_number(&self) -> u64 {
        0
    }

    fn drive_name(&self) -> &str {
        &self.drive_name
    }

    fn status(&mut self) -> Result<MtxStatus, Error> {
        let drive_status = self.load_status()?;

        let mut drives = Vec::new();

        if let Some(current_tape) = &drive_status.current_tape {
            drives.push(DriveStatus {
                loaded_slot: None,
                status: ElementStatus::VolumeTag(current_tape.name.clone()),
                drive_serial_number: None,
                vendor: None,
                model: None,
                element_address: 0,
            });
        }

        // This implementation is lame, because we do not have fixed
        // slot-assignment here.

        let mut slots = Vec::new();
        let label_texts = self.online_media_label_texts()?;
        let max_slots = ((label_texts.len() + 7) / 8) * 8;

        for i in 0..max_slots {
            let status = if let Some(label_text) = label_texts.get(i) {
                ElementStatus::VolumeTag(label_text.clone())
            } else {
                ElementStatus::Empty
            };
            slots.push(StorageElementStatus {
                import_export: false,
                status,
                element_address: (i + 1) as u16,
            });
        }

        Ok(MtxStatus {
            drives,
            slots,
            transports: Vec::new(),
        })
    }

    fn transfer_media(&mut self, _from: u64, _to: u64) -> Result<MtxStatus, Error> {
        bail!("media transfer is not implemented!");
    }

    fn export_media(&mut self, _label_text: &str) -> Result<Option<u64>, Error> {
        bail!("media export is not implemented!");
    }

    fn load_media_from_slot(&mut self, slot: u64) -> Result<MtxStatus, Error> {
        if slot < 1 {
            bail!("invalid slot ID {}", slot);
        }

        let label_texts = self.online_media_label_texts()?;

        if slot > label_texts.len() as u64 {
            bail!("slot {} is empty", slot);
        }

        self.load_media(&label_texts[slot as usize - 1])
    }

    /// Try to load media
    ///
    /// We automatically create an empty virtual tape here (if it does
    /// not exist already)
    fn load_media(&mut self, label: &str) -> Result<MtxStatus, Error> {
        let name = format!("tape-{}.json", label);
        let mut path = self.path.clone();
        path.push(&name);
        if !path.exists() {
            eprintln!("unable to find tape {} - creating file {:?}", label, path);
            let index = TapeIndex { files: 0 };
            self.store_tape_index(label, &index)?;
        }

        let status = VirtualDriveStatus {
            current_tape: Some(VirtualTapeStatus {
                name: label.to_string(),
                pos: 0,
            }),
        };
        self.store_status(&status)?;

        self.status()
    }

    fn unload_media(&mut self, _target_slot: Option<u64>) -> Result<MtxStatus, Error> {
        // Note: we currently simply ignore target_slot
        self.eject_media()?;
        self.status()
    }

    fn clean_drive(&mut self) -> Result<MtxStatus, Error> {
        // do nothing
        self.status()
    }
}

impl MediaChange for VirtualTapeDrive {
    fn drive_number(&self) -> u64 {
        0
    }

    fn drive_name(&self) -> &str {
        &self.name
    }

    fn status(&mut self) -> Result<MtxStatus, Error> {
        let mut handle = open_virtual_tape_drive(self)?;
        handle.status()
    }

    fn transfer_media(&mut self, from: u64, to: u64) -> Result<MtxStatus, Error> {
        let mut handle = open_virtual_tape_drive(self)?;
        handle.transfer_media(from, to)
    }

    fn export_media(&mut self, label_text: &str) -> Result<Option<u64>, Error> {
        let mut handle = open_virtual_tape_drive(self)?;
        handle.export_media(label_text)
    }

    fn load_media_from_slot(&mut self, slot: u64) -> Result<MtxStatus, Error> {
        let mut handle = open_virtual_tape_drive(self)?;
        handle.load_media_from_slot(slot)
    }

    fn load_media(&mut self, label_text: &str) -> Result<MtxStatus, Error> {
        let mut handle = open_virtual_tape_drive(self)?;
        handle.load_media(label_text)
    }

    fn unload_media(&mut self, target_slot: Option<u64>) -> Result<MtxStatus, Error> {
        let mut handle = open_virtual_tape_drive(self)?;
        handle.unload_media(target_slot)
    }

    fn online_media_label_texts(&mut self) -> Result<Vec<String>, Error> {
        let handle = open_virtual_tape_drive(self)?;
        handle.online_media_label_texts()
    }

    fn clean_drive(&mut self) -> Result<MtxStatus, Error> {
        let mut handle = open_virtual_tape_drive(self)?;
        handle.clean_drive()
    }
}
