use std::path::Path;
use std::ffi::OsStr;
use std::convert::TryFrom;

use anyhow::{bail, format_err, Error};
use serde_json::Value;

use proxmox::{
    api::{
        api,
        RpcEnvironment,
        RpcEnvironmentType,
        Router,
        Permission,
        section_config::SectionConfigData,
    },
    tools::{
        Uuid,
        io::ReadExt,
        fs::{
            replace_file,
            CreateOptions,
        },
    },
};

use crate::{
    task_log,
    task::TaskState,
    tools::compute_file_csum,
    api2::types::{
        DATASTORE_SCHEMA,
        DRIVE_NAME_SCHEMA,
        UPID_SCHEMA,
        Authid,
        Userid,
    },
    config::{
        self,
        cached_user_info::CachedUserInfo,
        acl::{
            PRIV_DATASTORE_BACKUP,
            PRIV_DATASTORE_MODIFY,
            PRIV_TAPE_READ,
        },
    },
    backup::{
        archive_type,
        MANIFEST_BLOB_NAME,
        CryptMode,
        DataStore,
        BackupDir,
        DataBlob,
        BackupManifest,
        ArchiveType,
        IndexFile,
        DynamicIndexReader,
        FixedIndexReader,
    },
    server::{
        lookup_user_email,
        WorkerTask,
    },
    tape::{
        TAPE_STATUS_DIR,
        TapeRead,
        MediaId,
        MediaCatalog,
        MediaPool,
        Inventory,
        file_formats::{
            PROXMOX_BACKUP_MEDIA_LABEL_MAGIC_1_0,
            PROXMOX_BACKUP_SNAPSHOT_ARCHIVE_MAGIC_1_0,
            PROXMOX_BACKUP_MEDIA_SET_LABEL_MAGIC_1_0,
            PROXMOX_BACKUP_CONTENT_HEADER_MAGIC_1_0,
            PROXMOX_BACKUP_CHUNK_ARCHIVE_MAGIC_1_0,
            MediaContentHeader,
            ChunkArchiveDecoder,
        },
        drive::{
            TapeDriver,
            request_and_load_media,
            lock_tape_device,
            set_tape_device_state,
        },
    },
};

pub const ROUTER: Router = Router::new()
    .post(&API_METHOD_RESTORE);

#[api(
   input: {
        properties: {
            store: {
                schema: DATASTORE_SCHEMA,
            },
            drive: {
                schema: DRIVE_NAME_SCHEMA,
            },
            "media-set": {
                description: "Media set UUID.",
                type: String,
            },
            "notify-user": {
                type: Userid,
                optional: true,
            },
            owner: {
                type: Authid,
                optional: true,
            },
        },
    },
    returns: {
        schema: UPID_SCHEMA,
    },
    access: {
        // Note: parameters are no uri parameter, so we need to test inside function body
        description: "The user needs Tape.Read privilege on /tape/pool/{pool} \
                      and /tape/drive/{drive}, Datastore.Backup privilege on /datastore/{store}.",
        permission: &Permission::Anybody,
    },
)]
/// Restore data from media-set
pub fn restore(
    store: String,
    drive: String,
    media_set: String,
    notify_user: Option<Userid>,
    owner: Option<Authid>,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let user_info = CachedUserInfo::new()?;

    let privs = user_info.lookup_privs(&auth_id, &["datastore", &store]);
    if (privs & PRIV_DATASTORE_BACKUP) == 0 {
        bail!("no permissions on /datastore/{}", store);
    }

    if let Some(ref owner) = owner {
        let correct_owner = owner == &auth_id
                || (owner.is_token()
                    && !auth_id.is_token()
                    && owner.user() == auth_id.user());

        // same permission as changing ownership after syncing
        if !correct_owner && privs & PRIV_DATASTORE_MODIFY == 0 {
            bail!("no permission to restore as '{}'", owner);
        }
    }

    let privs = user_info.lookup_privs(&auth_id, &["tape", "drive", &drive]);
    if (privs & PRIV_TAPE_READ) == 0 {
        bail!("no permissions on /tape/drive/{}", drive);
    }

    let status_path = Path::new(TAPE_STATUS_DIR);
    let inventory = Inventory::load(status_path)?;

    let media_set_uuid = media_set.parse()?;

    let pool = inventory.lookup_media_set_pool(&media_set_uuid)?;

    let privs = user_info.lookup_privs(&auth_id, &["tape", "pool", &pool]);
    if (privs & PRIV_TAPE_READ) == 0 {
        bail!("no permissions on /tape/pool/{}", pool);
    }

    let datastore = DataStore::lookup_datastore(&store)?;

    let (drive_config, _digest) = config::drive::config()?;

    // early check/lock before starting worker
    let drive_lock = lock_tape_device(&drive_config, &drive)?;

    let to_stdout = rpcenv.env_type() == RpcEnvironmentType::CLI;

    let upid_str = WorkerTask::new_thread(
        "tape-restore",
        Some(store.clone()),
        auth_id.clone(),
        to_stdout,
        move |worker| {
            let _drive_lock = drive_lock; // keep lock guard

            set_tape_device_state(&drive, &worker.upid().to_string())?;

            let _lock = MediaPool::lock(status_path, &pool)?;

            let members = inventory.compute_media_set_members(&media_set_uuid)?;

            let media_list = members.media_list();

            let mut media_id_list = Vec::new();

            let mut encryption_key_fingerprint = None;

            for (seq_nr, media_uuid) in media_list.iter().enumerate() {
                match media_uuid {
                    None => {
                        bail!("media set {} is incomplete (missing member {}).", media_set_uuid, seq_nr);
                    }
                    Some(media_uuid) => {
                        let media_id = inventory.lookup_media(media_uuid).unwrap();
                        if let Some(ref set) = media_id.media_set_label { // always true here
                            if encryption_key_fingerprint.is_none() && set.encryption_key_fingerprint.is_some() {
                                encryption_key_fingerprint = set.encryption_key_fingerprint.clone();
                            }
                        }
                        media_id_list.push(media_id);
                    }
                }
            }

            task_log!(worker, "Restore mediaset '{}'", media_set);
            if let Some(fingerprint) = encryption_key_fingerprint {
                task_log!(worker, "Encryption key fingerprint: {}", fingerprint);
            }
            task_log!(worker, "Pool: {}", pool);
            task_log!(worker, "Datastore: {}", store);
            task_log!(worker, "Drive: {}", drive);
            task_log!(
                worker,
                "Required media list: {}",
                media_id_list.iter()
                    .map(|media_id| media_id.label.label_text.as_str())
                    .collect::<Vec<&str>>()
                    .join(";")
            );

            for media_id in media_id_list.iter() {
                request_and_restore_media(
                    &worker,
                    media_id,
                    &drive_config,
                    &drive,
                    &datastore,
                    &auth_id,
                    &notify_user,
                    &owner,
                )?;
            }

            task_log!(worker, "Restore mediaset '{}' done", media_set);

            if let Err(err) = set_tape_device_state(&drive, "") {
                task_log!(
                    worker,
                    "could not unset drive state for {}: {}",
                    drive,
                    err
                );
            }

            Ok(())
        }
    )?;

    Ok(upid_str.into())
}

/// Request and restore complete media without using existing catalog (create catalog instead)
pub fn request_and_restore_media(
    worker: &WorkerTask,
    media_id: &MediaId,
    drive_config: &SectionConfigData,
    drive_name: &str,
    datastore: &DataStore,
    authid: &Authid,
    notify_user: &Option<Userid>,
    owner: &Option<Authid>,
) -> Result<(), Error> {

    let media_set_uuid = match media_id.media_set_label {
        None => bail!("restore_media: no media set - internal error"),
        Some(ref set) => &set.uuid,
    };

    let email = notify_user
        .as_ref()
        .and_then(|userid| lookup_user_email(userid))
        .or_else(|| lookup_user_email(&authid.clone().into()));

    let (mut drive, info) = request_and_load_media(worker, &drive_config, &drive_name, &media_id.label, &email)?;

    match info.media_set_label {
        None => {
            bail!("missing media set label on media {} ({})",
                  media_id.label.label_text, media_id.label.uuid);
        }
        Some(ref set) => {
            if &set.uuid != media_set_uuid {
                bail!("wrong media set label on media {} ({} != {})",
                      media_id.label.label_text, media_id.label.uuid,
                      media_set_uuid);
            }
            let encrypt_fingerprint = set.encryption_key_fingerprint.clone()
                .map(|fp| (fp, set.uuid.clone()));

            drive.set_encryption(encrypt_fingerprint)?;
        }
    }

    let restore_owner = owner.as_ref().unwrap_or(authid);

    restore_media(worker, &mut drive, &info, Some((datastore, restore_owner)), false)
}

/// Restore complete media content and catalog
///
/// Only create the catalog if target is None.
pub fn restore_media(
    worker: &WorkerTask,
    drive: &mut Box<dyn TapeDriver>,
    media_id: &MediaId,
    target: Option<(&DataStore, &Authid)>,
    verbose: bool,
) ->  Result<(), Error> {

    let status_path = Path::new(TAPE_STATUS_DIR);
    let mut catalog = MediaCatalog::create_temporary_database(status_path, media_id, false)?;

    loop {
        let current_file_number = drive.current_file_number()?;
        let reader = match drive.read_next_file()? {
            None => {
                task_log!(worker, "detected EOT after {} files", current_file_number);
                break;
            }
            Some(reader) => reader,
        };

        restore_archive(worker, reader, current_file_number, target, &mut catalog, verbose)?;
    }

    MediaCatalog::finish_temporary_database(status_path, &media_id.label.uuid, true)?;

    Ok(())
}

fn restore_archive<'a>(
    worker: &WorkerTask,
    mut reader: Box<dyn 'a + TapeRead>,
    current_file_number: u64,
    target: Option<(&DataStore, &Authid)>,
    catalog: &mut MediaCatalog,
    verbose: bool,
) -> Result<(), Error> {

    let header: MediaContentHeader = unsafe { reader.read_le_value()? };
    if header.magic != PROXMOX_BACKUP_CONTENT_HEADER_MAGIC_1_0 {
        bail!("missing MediaContentHeader");
    }

    //println!("Found MediaContentHeader: {:?}", header);

    match header.content_magic {
        PROXMOX_BACKUP_MEDIA_LABEL_MAGIC_1_0 | PROXMOX_BACKUP_MEDIA_SET_LABEL_MAGIC_1_0 => {
            bail!("unexpected content magic (label)");
        }
        PROXMOX_BACKUP_SNAPSHOT_ARCHIVE_MAGIC_1_0 => {
            let snapshot = reader.read_exact_allocated(header.size as usize)?;
            let snapshot = std::str::from_utf8(&snapshot)
                .map_err(|_| format_err!("found snapshot archive with non-utf8 characters in name"))?;
            task_log!(worker, "Found snapshot archive: {} {}", current_file_number, snapshot);

            let backup_dir: BackupDir = snapshot.parse()?;

            if let Some((datastore, authid)) = target.as_ref() {

                let (owner, _group_lock) = datastore.create_locked_backup_group(backup_dir.group(), authid)?;
                if *authid != &owner { // only the owner is allowed to create additional snapshots
                    bail!("restore '{}' failed - owner check failed ({} != {})", snapshot, authid, owner);
                }

                let (rel_path, is_new, _snap_lock) = datastore.create_locked_backup_dir(&backup_dir)?;
                let mut path = datastore.base_path();
                path.push(rel_path);

                if is_new {
                    task_log!(worker, "restore snapshot {}", backup_dir);

                    match restore_snapshot_archive(worker, reader, &path) {
                        Err(err) => {
                            std::fs::remove_dir_all(&path)?;
                            bail!("restore snapshot {} failed - {}", backup_dir, err);
                        }
                        Ok(false) => {
                            std::fs::remove_dir_all(&path)?;
                            task_log!(worker, "skip incomplete snapshot {}", backup_dir);
                        }
                        Ok(true) => {
                            catalog.register_snapshot(Uuid::from(header.uuid), current_file_number, snapshot)?;
                            catalog.commit_if_large()?;
                        }
                    }
                    return Ok(());
                }
            }

            reader.skip_to_end()?; // read all data
            if let Ok(false) = reader.is_incomplete() {
                catalog.register_snapshot(Uuid::from(header.uuid), current_file_number, snapshot)?;
                catalog.commit_if_large()?;
            }
        }
        PROXMOX_BACKUP_CHUNK_ARCHIVE_MAGIC_1_0 => {

            task_log!(worker, "Found chunk archive: {}", current_file_number);
            let datastore = target.as_ref().map(|t| t.0);

            if let Some(chunks) = restore_chunk_archive(worker, reader, datastore, verbose)? {
                catalog.start_chunk_archive(Uuid::from(header.uuid), current_file_number)?;
                for digest in chunks.iter() {
                    catalog.register_chunk(&digest)?;
                }
                task_log!(worker, "register {} chunks", chunks.len());
                catalog.end_chunk_archive()?;
                catalog.commit_if_large()?;
            }
        }
        _ =>  bail!("unknown content magic {:?}", header.content_magic),
    }

    catalog.commit()?;

    Ok(())
}

fn restore_chunk_archive<'a>(
    worker: &WorkerTask,
    reader: Box<dyn 'a + TapeRead>,
    datastore: Option<&DataStore>,
    verbose: bool,
) -> Result<Option<Vec<[u8;32]>>, Error> {

     let mut chunks = Vec::new();

    let mut decoder = ChunkArchiveDecoder::new(reader);

    let result: Result<_, Error> = proxmox::try_block!({
        while let Some((digest, blob)) = decoder.next_chunk()? {

            worker.check_abort()?;

            if let Some(datastore) = datastore {
                let chunk_exists = datastore.cond_touch_chunk(&digest, false)?;
                if !chunk_exists {
                    blob.verify_crc()?;

                    if blob.crypt_mode()? == CryptMode::None {
                        blob.decode(None, Some(&digest))?; // verify digest
                    }
                    if verbose {
                        task_log!(worker, "Insert chunk: {}", proxmox::tools::digest_to_hex(&digest));
                    }
                    datastore.insert_chunk(&blob, &digest)?;
                } else if verbose {
                    task_log!(worker, "Found existing chunk: {}", proxmox::tools::digest_to_hex(&digest));
                }
            } else if verbose {
                task_log!(worker, "Found chunk: {}", proxmox::tools::digest_to_hex(&digest));
            }
            chunks.push(digest);
        }
        Ok(())
    });

    match result {
        Ok(()) => Ok(Some(chunks)),
        Err(err) => {
            let reader = decoder.reader();

            // check if this stream is marked incomplete
            if let Ok(true) = reader.is_incomplete() {
                return Ok(Some(chunks));
            }

            // check if this is an aborted stream without end marker
            if let Ok(false) = reader.has_end_marker() {
                worker.log("missing stream end marker".to_string());
                return Ok(None);
            }

            // else the archive is corrupt
            Err(err)
        }
    }
}

fn restore_snapshot_archive<'a>(
    worker: &WorkerTask,
    reader: Box<dyn 'a + TapeRead>,
    snapshot_path: &Path,
) -> Result<bool, Error> {

    let mut decoder = pxar::decoder::sync::Decoder::from_std(reader)?;
    match try_restore_snapshot_archive(worker, &mut decoder, snapshot_path) {
        Ok(()) => Ok(true),
        Err(err) => {
            let reader = decoder.input();

            // check if this stream is marked incomplete
            if let Ok(true) = reader.is_incomplete() {
                return Ok(false);
            }

            // check if this is an aborted stream without end marker
            if let Ok(false) = reader.has_end_marker() {
                return Ok(false);
            }

            // else the archive is corrupt
            Err(err)
        }
    }
}

fn try_restore_snapshot_archive<R: pxar::decoder::SeqRead>(
    worker: &WorkerTask,
    decoder: &mut pxar::decoder::sync::Decoder<R>,
    snapshot_path: &Path,
) -> Result<(), Error> {

    let _root = match decoder.next() {
        None => bail!("missing root entry"),
        Some(root) => {
            let root = root?;
            match root.kind() {
                pxar::EntryKind::Directory => { /* Ok */ }
                _ => bail!("wrong root entry type"),
            }
            root
        }
    };

    let root_path = Path::new("/");
    let manifest_file_name = OsStr::new(MANIFEST_BLOB_NAME);

    let mut manifest = None;

    loop {
        worker.check_abort()?;

        let entry = match decoder.next() {
            None => break,
            Some(entry) => entry?,
        };
        let entry_path = entry.path();

        match entry.kind() {
            pxar::EntryKind::File { .. } => { /* Ok */ }
            _ => bail!("wrong entry type for {:?}", entry_path),
        }
        match entry_path.parent() {
            None => bail!("wrong parent for {:?}", entry_path),
            Some(p) => {
                if p != root_path {
                    bail!("wrong parent for {:?}", entry_path);
                }
            }
        }

        let filename = entry.file_name();
        let mut contents = match decoder.contents() {
            None => bail!("missing file content"),
            Some(contents) => contents,
        };

        let mut archive_path = snapshot_path.to_owned();
        archive_path.push(&filename);

        let mut tmp_path = archive_path.clone();
        tmp_path.set_extension("tmp");

        if filename == manifest_file_name {

            let blob = DataBlob::load_from_reader(&mut contents)?;
            let options = CreateOptions::new();
            replace_file(&tmp_path, blob.raw_data(), options)?;

            manifest = Some(BackupManifest::try_from(blob)?);
        } else {
            let mut tmpfile = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .read(true)
                .open(&tmp_path)
                .map_err(|err| format_err!("restore {:?} failed - {}", tmp_path, err))?;

            std::io::copy(&mut contents, &mut tmpfile)?;

            if let Err(err) = std::fs::rename(&tmp_path, &archive_path) {
                bail!("Atomic rename file {:?} failed - {}", archive_path, err);
            }
        }
    }

    let manifest = match manifest {
        None => bail!("missing manifest"),
        Some(manifest) => manifest,
    };

    for item in manifest.files() {
        let mut archive_path = snapshot_path.to_owned();
        archive_path.push(&item.filename);

        match archive_type(&item.filename)? {
            ArchiveType::DynamicIndex => {
                let index = DynamicIndexReader::open(&archive_path)?;
                let (csum, size) = index.compute_csum();
                manifest.verify_file(&item.filename, &csum, size)?;
            }
            ArchiveType::FixedIndex => {
                let index = FixedIndexReader::open(&archive_path)?;
                let (csum, size) = index.compute_csum();
                manifest.verify_file(&item.filename, &csum, size)?;
            }
            ArchiveType::Blob => {
                let mut tmpfile = std::fs::File::open(&archive_path)?;
                let (csum, size) = compute_file_csum(&mut tmpfile)?;
                manifest.verify_file(&item.filename, &csum, size)?;
            }
        }
    }

    // commit manifest
    let mut manifest_path = snapshot_path.to_owned();
    manifest_path.push(MANIFEST_BLOB_NAME);
    let mut tmp_manifest_path = manifest_path.clone();
    tmp_manifest_path.set_extension("tmp");

    if let Err(err) = std::fs::rename(&tmp_manifest_path, &manifest_path) {
        bail!("Atomic rename manifest {:?} failed - {}", manifest_path, err);
    }

    Ok(())
}
