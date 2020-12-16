use std::convert::TryFrom;
use std::fs::File;
use std::io::{Write, Read, BufReader, Seek, SeekFrom};
use std::os::unix::io::AsRawFd;
use std::path::Path;
use std::collections::HashMap;

use anyhow::{bail, format_err, Error};
use endian_trait::Endian;

use proxmox::tools::{
    Uuid,
    fs::{
        fchown,
        create_path,
        CreateOptions,
    },
    io::{
        WriteExt,
        ReadExt,
    },
};

use crate::{
    backup::BackupDir,
    tape::drive::MediaLabelInfo,
};

// openssl::sha::sha256(b"Proxmox Backup Media Catalog v1.0")[0..8]
pub const PROXMOX_BACKUP_MEDIA_CATALOG_MAGIC_1_0: [u8; 8] = [221, 29, 164, 1, 59, 69, 19, 40];

/// The Media Catalog
///
/// Stores what chunks and snapshots are stored on a specific media,
/// including the file position.
///
/// We use a simple binary format to store data on disk.
pub struct MediaCatalog  {

    uuid: Uuid, // BackupMedia uuid

    file: Option<File>,

    pub log_to_stdout: bool,

    current_archive: Option<(Uuid, u64)>,

    last_entry: Option<(Uuid, u64)>,

    chunk_index: HashMap<[u8;32], u64>,

    snapshot_index: HashMap<String, u64>,

    pending: Vec<u8>,
}

impl MediaCatalog {

    /// Test if a catalog exists
    pub fn exists(base_path: &Path, uuid: &Uuid) -> bool {
        let mut path = base_path.to_owned();
        path.push(uuid.to_string());
        path.set_extension("log");
        path.exists()
    }

    /// Destroy the media catalog (remove all files)
    pub fn destroy(base_path: &Path, uuid: &Uuid) -> Result<(), Error> {

        let mut path = base_path.to_owned();
        path.push(uuid.to_string());
        path.set_extension("log");

        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(err.into()),
        }
    }

    fn create_basedir(base_path: &Path) -> Result<(), Error> {
        let backup_user = crate::backup::backup_user()?;
        let mode = nix::sys::stat::Mode::from_bits_truncate(0o0640);
        let opts = CreateOptions::new()
            .perm(mode)
            .owner(backup_user.uid)
            .group(backup_user.gid);

        create_path(base_path, None, Some(opts))
            .map_err(|err: Error| format_err!("unable to create media catalog dir - {}", err))?;
        Ok(())
    }

    /// Open a catalog database, load into memory
    pub fn open(
        base_path: &Path,
        uuid: &Uuid,
        write: bool,
        create: bool,
    ) -> Result<Self, Error> {

        let mut path = base_path.to_owned();
        path.push(uuid.to_string());
        path.set_extension("log");

        let me = proxmox::try_block!({

            Self::create_basedir(base_path)?;

            let mut file = std::fs::OpenOptions::new()
                .read(true)
                .write(write)
                .create(create)
                .open(&path)?;

            let backup_user = crate::backup::backup_user()?;
            fchown(file.as_raw_fd(), Some(backup_user.uid), Some(backup_user.gid))
                .map_err(|err| format_err!("fchown failed - {}", err))?;

            let mut me = Self {
                uuid: uuid.clone(),
                file: None,
                log_to_stdout: false,
                current_archive: None,
                last_entry: None,
                chunk_index: HashMap::new(),
                snapshot_index: HashMap::new(),
                pending: Vec::new(),
            };

            let found_magic_number = me.load_catalog(&mut file)?;

            if !found_magic_number {
                me.pending.extend(&PROXMOX_BACKUP_MEDIA_CATALOG_MAGIC_1_0);
            }

            if write {
                me.file = Some(file);
            }
            Ok(me)
        }).map_err(|err: Error| {
            format_err!("unable to open media catalog {:?} - {}", path, err)
        })?;

        Ok(me)
    }

    /// Creates a temporary, empty catalog database
    pub fn create_temporary_database(
        base_path: &Path,
        label_info: &MediaLabelInfo,
        log_to_stdout: bool,
    ) -> Result<Self, Error> {

        let uuid = &label_info.label.uuid;

        let mut tmp_path = base_path.to_owned();
        tmp_path.push(uuid.to_string());
        tmp_path.set_extension("tmp");

        let me = proxmox::try_block!({

            Self::create_basedir(base_path)?;

            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(true)
                .open(&tmp_path)?;

            let backup_user = crate::backup::backup_user()?;
            fchown(file.as_raw_fd(), Some(backup_user.uid), Some(backup_user.gid))
                .map_err(|err| format_err!("fchown failed - {}", err))?;

            let mut me = Self {
                uuid: uuid.clone(),
                file: Some(file),
                log_to_stdout: false,
                current_archive: None,
                last_entry: None,
                chunk_index: HashMap::new(),
                snapshot_index: HashMap::new(),
                pending: Vec::new(),
            };

            me.log_to_stdout = log_to_stdout;

            me.register_label(&label_info.label_uuid, 0)?;

            if let Some((_, ref content_uuid)) = label_info.media_set_label {
                me.register_label(&content_uuid, 1)?;
            }

            me.pending.extend(&PROXMOX_BACKUP_MEDIA_CATALOG_MAGIC_1_0);
            me.commit()?;

            Ok(me)
        }).map_err(|err: Error| {
            format_err!("unable to create temporary media catalog {:?} - {}", tmp_path, err)
        })?;

        Ok(me)
    }

    /// Commit or Abort a temporary catalog database
    pub fn finish_temporary_database(
        base_path: &Path,
        uuid: &Uuid,
        commit: bool,
    ) -> Result<(), Error> {

        let mut tmp_path = base_path.to_owned();
        tmp_path.push(uuid.to_string());
        tmp_path.set_extension("tmp");

        if commit {
            let mut catalog_path = tmp_path.clone();
            catalog_path.set_extension("log");

            if let Err(err) = std::fs::rename(&tmp_path, &catalog_path) {
                bail!("Atomic rename catalog {:?} failed - {}", catalog_path, err);
            }
        } else {
            std::fs::remove_file(&tmp_path)?;
        }
        Ok(())
    }

    /// Returns the BackupMedia uuid
    pub fn uuid(&self) -> &Uuid {
        &self.uuid
    }

    /// Accessor to content list
    pub fn snapshot_index(&self) -> &HashMap<String, u64> {
        &self.snapshot_index
    }

    /// Commit pending changes
    ///
    /// This is necessary to store changes persistently.
    ///
    /// Fixme: this should be atomic ...
    pub fn commit(&mut self) -> Result<(), Error> {

        if self.pending.is_empty() {
            return Ok(());
        }

        match self.file {
            Some(ref mut file) => {
                file.write_all(&self.pending)?;
                file.flush()?;
                file.sync_data()?;
            }
            None => bail!("media catalog not writable (opened read only)"),
        }

        self.pending = Vec::new();

        Ok(())
    }

    /// Conditionally commit if in pending data is large (> 1Mb)
    pub fn commit_if_large(&mut self) -> Result<(), Error> {
        if self.pending.len() > 1024*1024 {
            self.commit()?;
        }
        Ok(())
    }

    /// Destroy existing catalog, opens a new one
    pub fn overwrite(
        base_path: &Path,
        label_info: &MediaLabelInfo,
        log_to_stdout: bool,
    ) ->  Result<Self, Error> {

        let uuid = &label_info.label.uuid;

        let me = Self::create_temporary_database(base_path, &label_info, log_to_stdout)?;

        Self::finish_temporary_database(base_path, uuid, true)?;

        Ok(me)
    }

    /// Test if the catalog already contain a snapshot
    pub fn contains_snapshot(&self, snapshot: &str) -> bool {
        self.snapshot_index.contains_key(snapshot)
    }

    /// Returns the chunk archive file number
    pub fn lookup_snapshot(&self, snapshot: &str) -> Option<u64> {
        self.snapshot_index.get(snapshot).map(|n| *n)
    }

    /// Test if the catalog already contain a chunk
    pub fn contains_chunk(&self, digest: &[u8;32]) -> bool {
        self.chunk_index.contains_key(digest)
    }

    /// Returns the chunk archive file number
    pub fn lookup_chunk(&self, digest: &[u8;32]) -> Option<u64> {
        self.chunk_index.get(digest).map(|n| *n)
    }

    fn check_register_label(&self, file_number: u64) -> Result<(), Error> {

        if file_number >= 2 {
            bail!("register label failed: got wrong file number ({} >= 2)", file_number);
        }

        if self.current_archive.is_some() {
            bail!("register label failed: inside chunk archive");
        }

        let expected_file_number = match self.last_entry {
            Some((_, last_number)) => last_number + 1,
            None => 0,
        };

        if file_number != expected_file_number {
            bail!("register label failed: got unexpected file number ({} < {})",
                  file_number, expected_file_number);
        }
        Ok(())
    }

    /// Register media labels (file 0 and 1)
    pub fn register_label(
        &mut self,
        uuid: &Uuid, // Uuid form MediaContentHeader
        file_number: u64,
    ) -> Result<(), Error> {

        self.check_register_label(file_number)?;

        let entry = LabelEntry {
            file_number,
            uuid: *uuid.as_bytes(),
        };

        if self.log_to_stdout {
            println!("L|{}|{}", file_number, uuid.to_string());
        }

        self.pending.push(b'L');

        unsafe { self.pending.write_le_value(entry)?; }

        self.last_entry = Some((uuid.clone(), file_number));

        Ok(())
    }

    /// Register a chunk
    ///
    /// Only valid after start_chunk_archive.
    pub fn register_chunk(
        &mut self,
        digest: &[u8;32],
    ) -> Result<(), Error> {

        let file_number = match self.current_archive {
            None => bail!("register_chunk failed: no archive started"),
            Some((_, file_number)) => file_number,
        };

        if self.log_to_stdout {
            println!("C|{}", proxmox::tools::digest_to_hex(digest));
        }

        self.pending.push(b'C');
        self.pending.extend(digest);

        self.chunk_index.insert(*digest, file_number);

        Ok(())
    }

    fn check_start_chunk_archive(&self, file_number: u64) -> Result<(), Error> {

        if self.current_archive.is_some() {
            bail!("start_chunk_archive failed: already started");
        }

        if file_number < 2 {
            bail!("start_chunk_archive failed: got wrong file number ({} < 2)", file_number);
        }

        let expect_min_file_number = match self.last_entry {
            Some((_, last_number)) => last_number + 1,
            None => 0,
        };

        if file_number < expect_min_file_number {
            bail!("start_chunk_archive: got unexpected file number ({} < {})",
                  file_number, expect_min_file_number);
        }

        Ok(())
    }

    /// Start a chunk archive section
    pub fn start_chunk_archive(
        &mut self,
        uuid: Uuid, // Uuid form MediaContentHeader
        file_number: u64,
    ) -> Result<(), Error> {

        self.check_start_chunk_archive(file_number)?;

        let entry = ChunkArchiveStart {
            file_number,
            uuid: *uuid.as_bytes(),
        };

        if self.log_to_stdout {
            println!("A|{}|{}", file_number, uuid.to_string());
        }

        self.pending.push(b'A');

        unsafe { self.pending.write_le_value(entry)?; }

        self.current_archive = Some((uuid, file_number));

        Ok(())
    }

    fn check_end_chunk_archive(&self, uuid: &Uuid, file_number: u64) -> Result<(), Error> {

        match self.current_archive {
            None => bail!("end_chunk archive failed: not started"),
            Some((ref expected_uuid, expected_file_number)) => {
                if uuid != expected_uuid {
                    bail!("end_chunk_archive failed: got unexpected uuid");
                }
                if file_number != expected_file_number {
                    bail!("end_chunk_archive failed: got unexpected file number ({} != {})",
                          file_number, expected_file_number);
                }
            }
        }

        Ok(())
    }

    /// End a chunk archive section
    pub fn end_chunk_archive(&mut self) -> Result<(), Error> {

        match self.current_archive.take() {
            None => bail!("end_chunk_archive failed: not started"),
            Some((uuid, file_number)) => {

                let entry = ChunkArchiveEnd {
                    file_number,
                    uuid: *uuid.as_bytes(),
                };

                if self.log_to_stdout {
                    println!("E|{}|{}\n", file_number, uuid.to_string());
                }

                self.pending.push(b'E');

                unsafe { self.pending.write_le_value(entry)?; }

                self.last_entry = Some((uuid, file_number));
            }
        }

        Ok(())
    }

    fn check_register_snapshot(&self, file_number: u64, snapshot: &str) -> Result<(), Error> {

        if self.current_archive.is_some() {
            bail!("register_snapshot failed: inside chunk_archive");
        }

        if file_number < 2 {
            bail!("register_snapshot failed: got wrong file number ({} < 2)", file_number);
        }

        let expect_min_file_number = match self.last_entry {
            Some((_, last_number)) => last_number + 1,
            None => 0,
        };

        if file_number < expect_min_file_number {
            bail!("register_snapshot failed: got unexpected file number ({} < {})",
                  file_number, expect_min_file_number);
        }

        if let Err(err) = snapshot.parse::<BackupDir>() {
            bail!("register_snapshot failed: unable to parse snapshot '{}' - {}", snapshot, err);
        }

        Ok(())
    }

    /// Register a snapshot
    pub fn register_snapshot(
        &mut self,
        uuid: Uuid, // Uuid form MediaContentHeader
        file_number: u64,
        snapshot: &str,
    ) -> Result<(), Error> {

        self.check_register_snapshot(file_number, snapshot)?;

        let entry = SnapshotEntry {
            file_number,
            uuid: *uuid.as_bytes(),
            name_len: u16::try_from(snapshot.len())?,
        };

        if self.log_to_stdout {
            println!("S|{}|{}|{}", file_number, uuid.to_string(), snapshot);
        }

        self.pending.push(b'S');

        unsafe { self.pending.write_le_value(entry)?; }
        self.pending.extend(snapshot.as_bytes());

        self.snapshot_index.insert(snapshot.to_string(), file_number);

        self.last_entry = Some((uuid, file_number));

        Ok(())
    }

    fn load_catalog(&mut self, file: &mut File) -> Result<bool, Error> {

        let mut file = BufReader::new(file);
        let mut found_magic_number = false;

        loop {
            let pos = file.seek(SeekFrom::Current(0))?;

            if pos == 0 { // read/check magic number
                let mut magic = [0u8; 8];
                match file.read_exact_or_eof(&mut magic) {
                    Ok(false) => { /* EOF */ break; }
                    Ok(true) => { /* OK */ }
                    Err(err) => bail!("read failed - {}", err),
                }
                if magic != PROXMOX_BACKUP_MEDIA_CATALOG_MAGIC_1_0 {
                    bail!("wrong magic number");
                }
                found_magic_number = true;
                continue;
            }

            let mut entry_type = [0u8; 1];
            match file.read_exact_or_eof(&mut entry_type) {
                Ok(false) => { /* EOF */ break; }
                Ok(true) => { /* OK */ }
                Err(err) => bail!("read failed - {}", err),
            }

            match entry_type[0] {
                b'C' => {
                    let file_number = match self.current_archive {
                        None => bail!("register_chunk failed: no archive started"),
                        Some((_, file_number)) => file_number,
                    };
                    let mut digest = [0u8; 32];
                    file.read_exact(&mut digest)?;
                    self.chunk_index.insert(digest, file_number);
                }
                b'A' => {
                    let entry: ChunkArchiveStart = unsafe { file.read_le_value()? };
                    let file_number = entry.file_number;
                    let uuid = Uuid::from(entry.uuid);

                    self.check_start_chunk_archive(file_number)?;

                    self.current_archive = Some((uuid, file_number));
                }
                b'E' => {
                    let entry: ChunkArchiveEnd = unsafe { file.read_le_value()? };
                    let file_number = entry.file_number;
                    let uuid = Uuid::from(entry.uuid);

                    self.check_end_chunk_archive(&uuid, file_number)?;

                    self.current_archive = None;
                    self.last_entry = Some((uuid, file_number));
                }
                b'S' => {
                    let entry: SnapshotEntry = unsafe { file.read_le_value()? };
                    let file_number = entry.file_number;
                    let name_len = entry.name_len;
                    let uuid = Uuid::from(entry.uuid);

                    let snapshot = file.read_exact_allocated(name_len.into())?;
                    let snapshot = std::str::from_utf8(&snapshot)?;

                    self.check_register_snapshot(file_number, snapshot)?;

                    self.snapshot_index.insert(snapshot.to_string(), file_number);

                    self.last_entry = Some((uuid, file_number));
                }
                b'L' => {
                    let entry: LabelEntry = unsafe { file.read_le_value()? };
                    let file_number = entry.file_number;
                    let uuid = Uuid::from(entry.uuid);

                    self.check_register_label(file_number)?;

                    self.last_entry = Some((uuid, file_number));
                }
                _ => {
                    bail!("unknown entry type '{}'", entry_type[0]);
                }
            }

        }

        Ok(found_magic_number)
    }
}

/// Media set catalog
///
/// Catalog for multiple media.
pub struct MediaSetCatalog  {
    catalog_list: HashMap<Uuid, MediaCatalog>,
}

impl MediaSetCatalog {

    /// Creates a new instance
    pub fn new() -> Self {
        Self {
            catalog_list: HashMap::new(),
        }
    }

    /// Add a catalog
    pub fn append_catalog(&mut self, catalog: MediaCatalog) -> Result<(), Error> {

        if self.catalog_list.get(&catalog.uuid).is_some() {
            bail!("MediaSetCatalog already contains media '{}'", catalog.uuid);
        }

        self.catalog_list.insert(catalog.uuid.clone(), catalog);

        Ok(())
    }

    /// Remove a catalog
    pub fn remove_catalog(&mut self, media_uuid: &Uuid) {
        self.catalog_list.remove(media_uuid);
    }

    /// Test if the catalog already contain a snapshot
    pub fn contains_snapshot(&self, snapshot: &str) -> bool {
        for catalog in self.catalog_list.values() {
            if catalog.contains_snapshot(snapshot) {
                return true;
            }
        }
        false
    }

    /// Test if the catalog already contain a chunk
    pub fn contains_chunk(&self, digest: &[u8;32]) -> bool {
        for catalog in self.catalog_list.values() {
            if catalog.contains_chunk(digest) {
                return true;
            }
        }
        false
    }
}

// Type definitions for internal binary catalog encoding

#[derive(Endian)]
#[repr(C)]
pub struct LabelEntry {
    file_number: u64,
    uuid: [u8;16],
}

#[derive(Endian)]
#[repr(C)]
pub struct ChunkArchiveStart {
    file_number: u64,
    uuid: [u8;16],
}

#[derive(Endian)]
#[repr(C)]
pub struct ChunkArchiveEnd{
    file_number: u64,
    uuid: [u8;16],
}

#[derive(Endian)]
#[repr(C)]
pub struct SnapshotEntry{
    file_number: u64,
    uuid: [u8;16],
    name_len: u16,
    /* snapshot name follows */
}
