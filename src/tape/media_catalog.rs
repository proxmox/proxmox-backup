use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom, Write};
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};

use anyhow::{bail, format_err, Error};
use endian_trait::Endian;

use proxmox_sys::fs::read_subdir;

use proxmox_io::{ReadExt, WriteExt};
use proxmox_sys::fs::{create_path, fchown, CreateOptions};
use proxmox_uuid::Uuid;

use pbs_api_types::{parse_ns_and_snapshot, print_ns_and_snapshot, BackupDir, BackupNamespace};

use crate::tape::{file_formats::MediaSetLabel, MediaId};

#[derive(Default)]
pub struct DatastoreContent {
    pub snapshot_index: HashMap<String, u64>, // snapshot => file_nr
    pub chunk_index: HashMap<[u8; 32], u64>,  // chunk => file_nr
}

impl DatastoreContent {
    pub fn new() -> Self {
        Self {
            chunk_index: HashMap::new(),
            snapshot_index: HashMap::new(),
        }
    }
}

/// The Media Catalog
///
/// Stores what chunks and snapshots are stored on a specific media,
/// including the file position.
///
/// We use a simple binary format to store data on disk.
pub struct MediaCatalog {
    uuid: Uuid, // BackupMedia uuid

    file: Option<File>,

    log_to_stdout: bool,

    current_archive: Option<(Uuid, u64, String)>, // (uuid, file_nr, store)

    last_entry: Option<(Uuid, u64)>,

    content: HashMap<String, DatastoreContent>,

    pending: Vec<u8>,
}

impl MediaCatalog {
    /// Magic number for media catalog files.
    // openssl::sha::sha256(b"Proxmox Backup Media Catalog v1.0")[0..8]
    // Note: this version did not store datastore names (not supported anymore)
    pub const PROXMOX_BACKUP_MEDIA_CATALOG_MAGIC_1_0: [u8; 8] = [221, 29, 164, 1, 59, 69, 19, 40];

    // openssl::sha::sha256(b"Proxmox Backup Media Catalog v1.1")[0..8]
    pub const PROXMOX_BACKUP_MEDIA_CATALOG_MAGIC_1_1: [u8; 8] =
        [76, 142, 232, 193, 32, 168, 137, 113];

    /// List media with catalogs
    pub fn media_with_catalogs<P: AsRef<Path>>(base_path: P) -> Result<HashSet<Uuid>, Error> {
        let mut catalogs = HashSet::new();

        for entry in read_subdir(libc::AT_FDCWD, base_path.as_ref())? {
            let entry = entry?;
            let name = unsafe { entry.file_name_utf8_unchecked() };
            if !name.ends_with(".log") {
                continue;
            }
            if let Ok(uuid) = Uuid::parse_str(&name[..(name.len() - 4)]) {
                catalogs.insert(uuid);
            }
        }

        Ok(catalogs)
    }

    fn catalog_path<P: AsRef<Path>>(base_path: P, uuid: &Uuid) -> PathBuf {
        let mut path = base_path.as_ref().to_owned();
        path.push(uuid.to_string());
        path.set_extension("log");
        path
    }

    fn tmp_catalog_path<P: AsRef<Path>>(base_path: P, uuid: &Uuid) -> PathBuf {
        let mut path = base_path.as_ref().to_owned();
        path.push(uuid.to_string());
        path.set_extension("tmp");
        path
    }

    /// Test if a catalog exists
    pub fn exists<P: AsRef<Path>>(base_path: P, uuid: &Uuid) -> bool {
        Self::catalog_path(base_path, uuid).exists()
    }

    /// Destroy the media catalog (remove all files)
    pub fn destroy<P: AsRef<Path>>(base_path: P, uuid: &Uuid) -> Result<(), Error> {
        let path = Self::catalog_path(base_path, uuid);

        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(err.into()),
        }
    }

    /// Destroy the media catalog if media_set uuid does not match
    pub fn destroy_unrelated_catalog<P: AsRef<Path>>(
        base_path: P,
        media_id: &MediaId,
    ) -> Result<(), Error> {
        let uuid = &media_id.label.uuid;

        let path = Self::catalog_path(base_path, uuid);

        let file = match std::fs::OpenOptions::new().read(true).open(&path) {
            Ok(file) => file,
            Err(ref err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Ok(());
            }
            Err(err) => return Err(err.into()),
        };

        let mut file = BufReader::new(file);

        let expected_media_set_id = match media_id.media_set_label {
            None => {
                std::fs::remove_file(path)?;
                return Ok(());
            }
            Some(ref set) => &set.uuid,
        };

        let (found_magic_number, media_uuid, media_set_uuid) =
            Self::parse_catalog_header(&mut file)?;

        if !found_magic_number {
            return Ok(());
        }

        if let Some(ref media_uuid) = media_uuid {
            if media_uuid != uuid {
                std::fs::remove_file(path)?;
                return Ok(());
            }
        }

        if let Some(ref media_set_uuid) = media_set_uuid {
            if media_set_uuid != expected_media_set_id {
                std::fs::remove_file(path)?;
            }
        }

        Ok(())
    }

    /// Enable/Disable logging to stdout (disabled by default)
    pub fn log_to_stdout(&mut self, enable: bool) {
        self.log_to_stdout = enable;
    }

    fn create_basedir<P: AsRef<Path>>(base_path: P) -> Result<(), Error> {
        let backup_user = pbs_config::backup_user()?;
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
    pub fn open<P: AsRef<Path>>(
        base_path: P,
        media_id: &MediaId,
        write: bool,
        create: bool,
    ) -> Result<Self, Error> {
        let uuid = &media_id.label.uuid;

        let path = Self::catalog_path(&base_path, uuid);

        let me = proxmox_lang::try_block!({
            Self::create_basedir(base_path)?;

            let mut file = std::fs::OpenOptions::new()
                .read(true)
                .write(write)
                .create(create)
                .open(&path)?;

            let backup_user = pbs_config::backup_user()?;
            fchown(
                file.as_raw_fd(),
                Some(backup_user.uid),
                Some(backup_user.gid),
            )
            .map_err(|err| format_err!("fchown failed - {}", err))?;

            let mut me = Self {
                uuid: uuid.clone(),
                file: None,
                log_to_stdout: false,
                current_archive: None,
                last_entry: None,
                content: HashMap::new(),
                pending: Vec::new(),
            };

            // Note: lock file, to get a consistent view with load_catalog
            nix::fcntl::flock(file.as_raw_fd(), nix::fcntl::FlockArg::LockExclusive)?;
            let result = me.load_catalog(&mut file, media_id.media_set_label.as_ref());
            nix::fcntl::flock(file.as_raw_fd(), nix::fcntl::FlockArg::Unlock)?;

            let (found_magic_number, _) = result?;

            if !found_magic_number {
                me.pending
                    .extend(Self::PROXMOX_BACKUP_MEDIA_CATALOG_MAGIC_1_1);
            }

            if write {
                me.file = Some(file);
            }
            Ok(me)
        })
        .map_err(|err: Error| format_err!("unable to open media catalog {:?} - {}", path, err))?;

        Ok(me)
    }

    /// Creates a temporary empty catalog file
    pub fn create_temporary_database_file<P: AsRef<Path>>(
        base_path: P,
        uuid: &Uuid,
    ) -> Result<File, Error> {
        Self::create_basedir(&base_path)?;

        let tmp_path = Self::tmp_catalog_path(base_path, uuid);

        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(tmp_path)?;

        if cfg!(test) {
            // We cannot use chown inside test environment (no permissions)
            return Ok(file);
        }

        let backup_user = pbs_config::backup_user()?;
        fchown(
            file.as_raw_fd(),
            Some(backup_user.uid),
            Some(backup_user.gid),
        )
        .map_err(|err| format_err!("fchown failed - {}", err))?;

        Ok(file)
    }

    /// Creates a temporary, empty catalog database
    ///
    /// Creates a new catalog file using a ".tmp" file extension.
    pub fn create_temporary_database<P: AsRef<Path>>(
        base_path: P,
        media_id: &MediaId,
        log_to_stdout: bool,
    ) -> Result<Self, Error> {
        let uuid = &media_id.label.uuid;

        let tmp_path = Self::tmp_catalog_path(&base_path, uuid);

        let me = proxmox_lang::try_block!({
            let file = Self::create_temporary_database_file(base_path, uuid)?;

            let mut me = Self {
                uuid: uuid.clone(),
                file: Some(file),
                log_to_stdout: false,
                current_archive: None,
                last_entry: None,
                content: HashMap::new(),
                pending: Vec::new(),
            };

            me.log_to_stdout = log_to_stdout;

            me.pending
                .extend(Self::PROXMOX_BACKUP_MEDIA_CATALOG_MAGIC_1_1);

            me.register_label(&media_id.label.uuid, 0, 0)?;

            if let Some(ref set) = media_id.media_set_label {
                me.register_label(&set.uuid, set.seq_nr, 1)?;
            }

            me.commit()?;

            Ok(me)
        })
        .map_err(|err: Error| {
            format_err!(
                "unable to create temporary media catalog {:?} - {}",
                tmp_path,
                err
            )
        })?;

        Ok(me)
    }

    /// Commit or Abort a temporary catalog database
    ///
    /// With commit set, we rename the ".tmp" file extension to
    /// ".log". When commit is false, we remove the ".tmp" file.
    pub fn finish_temporary_database<P: AsRef<Path>>(
        base_path: P,
        uuid: &Uuid,
        commit: bool,
    ) -> Result<(), Error> {
        let tmp_path = Self::tmp_catalog_path(base_path, uuid);

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
    pub fn content(&self) -> &HashMap<String, DatastoreContent> {
        &self.content
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
                let pending = &self.pending;
                // Note: lock file, to get a consistent view with load_catalog
                nix::fcntl::flock(file.as_raw_fd(), nix::fcntl::FlockArg::LockExclusive)?;
                let result: Result<(), Error> = proxmox_lang::try_block!({
                    file.write_all(pending)?;
                    file.flush()?;
                    file.sync_data()?;
                    Ok(())
                });
                nix::fcntl::flock(file.as_raw_fd(), nix::fcntl::FlockArg::Unlock)?;

                result?;
            }
            None => bail!("media catalog not writable (opened read only)"),
        }

        self.pending = Vec::new();

        Ok(())
    }

    /// Conditionally commit if in pending data is large (> 1Mb)
    pub fn commit_if_large(&mut self) -> Result<(), Error> {
        if self.current_archive.is_some() {
            bail!("can't commit catalog in the middle of an chunk archive");
        }
        if self.pending.len() > 1024 * 1024 {
            self.commit()?;
        }
        Ok(())
    }

    /// Destroy existing catalog, opens a new one
    pub fn overwrite<P: AsRef<Path>>(
        base_path: P,
        media_id: &MediaId,
        log_to_stdout: bool,
    ) -> Result<Self, Error> {
        let uuid = &media_id.label.uuid;

        let me = Self::create_temporary_database(&base_path, media_id, log_to_stdout)?;

        Self::finish_temporary_database(base_path, uuid, true)?;

        Ok(me)
    }

    /// Test if the catalog already contain a snapshot
    pub fn contains_snapshot(
        &self,
        store: &str,
        ns: &BackupNamespace,
        snapshot: &BackupDir,
    ) -> bool {
        let path = print_ns_and_snapshot(ns, snapshot);
        match self.content.get(store) {
            None => false,
            Some(content) => content.snapshot_index.contains_key(&path),
        }
    }

    /// Returns the snapshot archive file number
    pub fn lookup_snapshot(&self, store: &str, snapshot: &str) -> Option<u64> {
        match self.content.get(store) {
            None => None,
            Some(content) => content.snapshot_index.get(snapshot).copied(),
        }
    }

    /// Test if the catalog already contain a chunk
    pub fn contains_chunk(&self, store: &str, digest: &[u8; 32]) -> bool {
        match self.content.get(store) {
            None => false,
            Some(content) => content.chunk_index.contains_key(digest),
        }
    }

    /// Returns the chunk archive file number
    pub fn lookup_chunk(&self, store: &str, digest: &[u8; 32]) -> Option<u64> {
        match self.content.get(store) {
            None => None,
            Some(content) => content.chunk_index.get(digest).copied(),
        }
    }

    fn check_register_label(&self, file_number: u64, uuid: &Uuid) -> Result<(), Error> {
        if file_number >= 2 {
            bail!(
                "register label failed: got wrong file number ({} >= 2)",
                file_number
            );
        }

        if file_number == 0 && uuid != &self.uuid {
            bail!("register label failed: uuid does not match");
        }

        if self.current_archive.is_some() {
            bail!("register label failed: inside chunk archive");
        }

        let expected_file_number = match self.last_entry {
            Some((_, last_number)) => last_number + 1,
            None => 0,
        };

        if file_number != expected_file_number {
            bail!(
                "register label failed: got unexpected file number ({} < {})",
                file_number,
                expected_file_number
            );
        }
        Ok(())
    }

    /// Register media labels (file 0 and 1)
    pub fn register_label(
        &mut self,
        uuid: &Uuid, // Media/MediaSet Uuid
        seq_nr: u64, // only used for media set labels
        file_number: u64,
    ) -> Result<(), Error> {
        self.check_register_label(file_number, uuid)?;

        if file_number == 0 && seq_nr != 0 {
            bail!("register_label failed - seq_nr should be 0 - iternal error");
        }

        let entry = LabelEntry {
            file_number,
            uuid: *uuid.as_bytes(),
            seq_nr,
        };

        if self.log_to_stdout {
            println!("L|{}|{}", file_number, uuid);
        }

        self.pending.push(b'L');

        unsafe {
            self.pending.write_le_value(entry)?;
        }

        self.last_entry = Some((uuid.clone(), file_number));

        Ok(())
    }

    /// Register a chunk archive
    pub fn register_chunk_archive(
        &mut self,
        uuid: Uuid, // Uuid form MediaContentHeader
        file_number: u64,
        store: &str,
        chunk_list: &[[u8; 32]],
    ) -> Result<(), Error> {
        self.start_chunk_archive(uuid, file_number, store)?;
        for digest in chunk_list {
            self.register_chunk(digest)?;
        }
        self.end_chunk_archive()?;
        Ok(())
    }

    /// Register a chunk
    ///
    /// Only valid after start_chunk_archive.
    fn register_chunk(&mut self, digest: &[u8; 32]) -> Result<(), Error> {
        let (file_number, store) = match self.current_archive {
            None => bail!("register_chunk failed: no archive started"),
            Some((_, file_number, ref store)) => (file_number, store),
        };

        if self.log_to_stdout {
            println!("C|{}", hex::encode(digest));
        }

        self.pending.push(b'C');
        self.pending.extend(digest);

        match self.content.get_mut(store) {
            None => bail!("storage {} not registered - internal error", store),
            Some(content) => {
                content.chunk_index.insert(*digest, file_number);
            }
        }

        Ok(())
    }

    fn check_start_chunk_archive(&self, file_number: u64) -> Result<(), Error> {
        if self.current_archive.is_some() {
            bail!("start_chunk_archive failed: already started");
        }

        if file_number < 2 {
            bail!(
                "start_chunk_archive failed: got wrong file number ({} < 2)",
                file_number
            );
        }

        let expect_min_file_number = match self.last_entry {
            Some((_, last_number)) => last_number + 1,
            None => 0,
        };

        if file_number < expect_min_file_number {
            bail!(
                "start_chunk_archive: got unexpected file number ({} < {})",
                file_number,
                expect_min_file_number
            );
        }

        Ok(())
    }

    /// Start a chunk archive section
    fn start_chunk_archive(
        &mut self,
        uuid: Uuid, // Uuid form MediaContentHeader
        file_number: u64,
        store: &str,
    ) -> Result<(), Error> {
        self.check_start_chunk_archive(file_number)?;

        let entry = ChunkArchiveStart {
            file_number,
            uuid: *uuid.as_bytes(),
            store_name_len: u8::try_from(store.len())?,
        };

        if self.log_to_stdout {
            println!("A|{}|{}|{}", file_number, uuid, store);
        }

        self.pending.push(b'A');

        unsafe {
            self.pending.write_le_value(entry)?;
        }
        self.pending.extend(store.as_bytes());

        self.content.entry(store.to_string()).or_default();

        self.current_archive = Some((uuid, file_number, store.to_string()));

        Ok(())
    }

    fn check_end_chunk_archive(&self, uuid: &Uuid, file_number: u64) -> Result<(), Error> {
        match self.current_archive {
            None => bail!("end_chunk archive failed: not started"),
            Some((ref expected_uuid, expected_file_number, ..)) => {
                if uuid != expected_uuid {
                    bail!("end_chunk_archive failed: got unexpected uuid");
                }
                if file_number != expected_file_number {
                    bail!(
                        "end_chunk_archive failed: got unexpected file number ({} != {})",
                        file_number,
                        expected_file_number
                    );
                }
            }
        }
        Ok(())
    }

    /// End a chunk archive section
    fn end_chunk_archive(&mut self) -> Result<(), Error> {
        match self.current_archive.take() {
            None => bail!("end_chunk_archive failed: not started"),
            Some((uuid, file_number, ..)) => {
                let entry = ChunkArchiveEnd {
                    file_number,
                    uuid: *uuid.as_bytes(),
                };

                if self.log_to_stdout {
                    println!("E|{}|{}\n", file_number, uuid);
                }

                self.pending.push(b'E');

                unsafe {
                    self.pending.write_le_value(entry)?;
                }

                self.last_entry = Some((uuid, file_number));
            }
        }

        Ok(())
    }

    fn check_register_snapshot(&self, file_number: u64) -> Result<(), Error> {
        if self.current_archive.is_some() {
            bail!("register_snapshot failed: inside chunk_archive");
        }

        if file_number < 2 {
            bail!(
                "register_snapshot failed: got wrong file number ({} < 2)",
                file_number
            );
        }

        let expect_min_file_number = match self.last_entry {
            Some((_, last_number)) => last_number + 1,
            None => 0,
        };

        if file_number < expect_min_file_number {
            bail!(
                "register_snapshot failed: got unexpected file number ({} < {})",
                file_number,
                expect_min_file_number
            );
        }

        Ok(())
    }

    /// Register a snapshot
    pub fn register_snapshot(
        &mut self,
        uuid: Uuid, // Uuid form MediaContentHeader
        file_number: u64,
        store: &str,
        ns: &BackupNamespace,
        snapshot: &BackupDir,
    ) -> Result<(), Error> {
        self.check_register_snapshot(file_number)?;

        let path = print_ns_and_snapshot(ns, snapshot);

        let entry = SnapshotEntry {
            file_number,
            uuid: *uuid.as_bytes(),
            store_name_len: u8::try_from(store.len())?,
            name_len: u16::try_from(path.len())?,
        };

        if self.log_to_stdout {
            println!("S|{}|{}|{}:{}", file_number, uuid, store, path,);
        }

        self.pending.push(b'S');

        unsafe {
            self.pending.write_le_value(entry)?;
        }
        self.pending.extend(store.as_bytes());
        self.pending.push(b':');
        self.pending.extend(path.as_bytes());

        let content = self.content.entry(store.to_string()).or_default();

        content.snapshot_index.insert(path, file_number);

        self.last_entry = Some((uuid, file_number));

        Ok(())
    }

    /// Parse the catalog header
    pub fn parse_catalog_header<R: Read>(
        reader: &mut R,
    ) -> Result<(bool, Option<Uuid>, Option<Uuid>), Error> {
        // read/check magic number
        let mut magic = [0u8; 8];
        if !reader.read_exact_or_eof(&mut magic)? {
            /* EOF */
            return Ok((false, None, None));
        }

        match magic {
            // only used in unreleased versions
            Self::PROXMOX_BACKUP_MEDIA_CATALOG_MAGIC_1_0 => {
                bail!("old catalog format (v1.0) is no longer supported")
            }
            Self::PROXMOX_BACKUP_MEDIA_CATALOG_MAGIC_1_1 => {}
            _ => bail!("wrong magic number"),
        }

        let mut entry_type = [0u8; 1];
        if !reader.read_exact_or_eof(&mut entry_type)? {
            /* EOF */
            return Ok((true, None, None));
        }

        if entry_type[0] != b'L' {
            bail!("got unexpected entry type");
        }

        let entry0: LabelEntry = unsafe { reader.read_le_value()? };

        let mut entry_type = [0u8; 1];
        if !reader.read_exact_or_eof(&mut entry_type)? {
            /* EOF */
            return Ok((true, Some(entry0.uuid.into()), None));
        }

        if entry_type[0] != b'L' {
            bail!("got unexpected entry type");
        }

        let entry1: LabelEntry = unsafe { reader.read_le_value()? };

        Ok((true, Some(entry0.uuid.into()), Some(entry1.uuid.into())))
    }

    fn load_catalog(
        &mut self,
        file: &mut File,
        media_set_label: Option<&MediaSetLabel>,
    ) -> Result<(bool, Option<Uuid>), Error> {
        let mut file = BufReader::new(file);
        let mut found_magic_number = false;
        let mut media_set_uuid = None;

        loop {
            let pos = file.seek(SeekFrom::Current(0))?; // get current pos

            if pos == 0 {
                // read/check magic number
                let mut magic = [0u8; 8];
                match file.read_exact_or_eof(&mut magic) {
                    Ok(false) => {
                        /* EOF */
                        break;
                    }
                    Ok(true) => { /* OK */ }
                    Err(err) => bail!("read failed - {}", err),
                }

                match magic {
                    // only used in unreleased versions
                    Self::PROXMOX_BACKUP_MEDIA_CATALOG_MAGIC_1_0 => {
                        bail!("old catalog format (v1.0) is no longer supported")
                    }
                    Self::PROXMOX_BACKUP_MEDIA_CATALOG_MAGIC_1_1 => {}
                    _ => bail!("wrong magic number"),
                }
                found_magic_number = true;
                continue;
            }

            let mut entry_type = [0u8; 1];
            match file.read_exact_or_eof(&mut entry_type) {
                Ok(false) => {
                    /* EOF */
                    break;
                }
                Ok(true) => { /* OK */ }
                Err(err) => bail!("read failed - {}", err),
            }

            match entry_type[0] {
                b'C' => {
                    let (file_number, store) = match self.current_archive {
                        None => bail!("register_chunk failed: no archive started"),
                        Some((_, file_number, ref store)) => (file_number, store),
                    };
                    let mut digest = [0u8; 32];
                    file.read_exact(&mut digest)?;
                    match self.content.get_mut(store) {
                        None => bail!("storage {} not registered - internal error", store),
                        Some(content) => {
                            content.chunk_index.insert(digest, file_number);
                        }
                    }
                }
                b'A' => {
                    let entry: ChunkArchiveStart = unsafe { file.read_le_value()? };
                    let file_number = entry.file_number;
                    let uuid = Uuid::from(entry.uuid);
                    let store_name_len = entry.store_name_len as usize;

                    let store = file.read_exact_allocated(store_name_len)?;
                    let store = std::str::from_utf8(&store)?;

                    self.check_start_chunk_archive(file_number)?;

                    self.content.entry(store.to_string()).or_default();

                    self.current_archive = Some((uuid, file_number, store.to_string()));
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
                    let store_name_len = entry.store_name_len as usize;
                    let name_len = entry.name_len as usize;
                    let uuid = Uuid::from(entry.uuid);

                    let store = file.read_exact_allocated(store_name_len + 1)?;
                    if store[store_name_len] != b':' {
                        bail!("parse-error: missing separator in SnapshotEntry");
                    }

                    let store = std::str::from_utf8(&store[..store_name_len])?;

                    let snapshot = file.read_exact_allocated(name_len)?;
                    let snapshot = std::str::from_utf8(&snapshot)?;

                    let _ = parse_ns_and_snapshot(snapshot)?;
                    self.check_register_snapshot(file_number)?;

                    let content = self.content.entry(store.to_string()).or_default();

                    content
                        .snapshot_index
                        .insert(snapshot.to_string(), file_number);

                    self.last_entry = Some((uuid, file_number));
                }
                b'L' => {
                    let entry: LabelEntry = unsafe { file.read_le_value()? };
                    let file_number = entry.file_number;
                    let uuid = Uuid::from(entry.uuid);

                    self.check_register_label(file_number, &uuid)?;

                    if file_number == 1 {
                        if let Some(set) = media_set_label {
                            if set.uuid != uuid {
                                bail!("got unexpected media set uuid");
                            }
                            if set.seq_nr != entry.seq_nr {
                                bail!("got unexpected media set sequence number");
                            }
                        }
                        media_set_uuid = Some(uuid.clone());
                    }

                    self.last_entry = Some((uuid, file_number));
                }
                _ => {
                    bail!("unknown entry type '{}'", entry_type[0]);
                }
            }
        }

        Ok((found_magic_number, media_set_uuid))
    }
}

/// Media set catalog
///
/// Catalog for multiple media.
#[derive(Default)]
pub struct MediaSetCatalog {
    catalog_list: HashMap<Uuid, MediaCatalog>,
}

impl MediaSetCatalog {
    /// Creates a new instance
    pub fn new() -> Self {
        Self::default()
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
    pub fn contains_snapshot(
        &self,
        store: &str,
        ns: &BackupNamespace,
        snapshot: &BackupDir,
    ) -> bool {
        for catalog in self.catalog_list.values() {
            if catalog.contains_snapshot(store, ns, snapshot) {
                return true;
            }
        }
        false
    }

    /// Returns the media uuid and snapshot archive file number
    pub fn lookup_snapshot(&self, store: &str, snapshot: &str) -> Option<(&Uuid, u64)> {
        for (uuid, catalog) in self.catalog_list.iter() {
            if let Some(nr) = catalog.lookup_snapshot(store, snapshot) {
                return Some((uuid, nr));
            }
        }
        None
    }

    /// Test if the catalog already contain a chunk
    pub fn contains_chunk(&self, store: &str, digest: &[u8; 32]) -> bool {
        for catalog in self.catalog_list.values() {
            if catalog.contains_chunk(store, digest) {
                return true;
            }
        }
        false
    }

    /// Returns the media uuid and chunk archive file number
    pub fn lookup_chunk(&self, store: &str, digest: &[u8; 32]) -> Option<(&Uuid, u64)> {
        for (uuid, catalog) in self.catalog_list.iter() {
            if let Some(nr) = catalog.lookup_chunk(store, digest) {
                return Some((uuid, nr));
            }
        }
        None
    }

    /// Returns an iterator over all registered snapshots per datastore
    /// as (datastore, snapshot).
    /// The snapshot contains namespaces in the format 'ns/namespace'.
    pub fn list_snapshots(&self) -> impl Iterator<Item = (&str, &str)> {
        self.catalog_list.values().flat_map(|catalog| {
            catalog.content.iter().flat_map(|(store, content)| {
                content
                    .snapshot_index
                    .keys()
                    .map(move |key| (store.as_str(), key.as_str()))
            })
        })
    }
}

// Type definitions for internal binary catalog encoding

#[derive(Endian)]
#[repr(C)]
struct LabelEntry {
    file_number: u64,
    uuid: [u8; 16],
    seq_nr: u64, // only used for media set labels
}

#[derive(Endian)]
#[repr(C)]
struct ChunkArchiveStart {
    file_number: u64,
    uuid: [u8; 16],
    store_name_len: u8,
    /* datastore name follows */
}

#[derive(Endian)]
#[repr(C)]
struct ChunkArchiveEnd {
    file_number: u64,
    uuid: [u8; 16],
}

#[derive(Endian)]
#[repr(C)]
struct SnapshotEntry {
    file_number: u64,
    uuid: [u8; 16],
    store_name_len: u8,
    name_len: u16,
    /* datastore name,  ':', snapshot name follows */
}
