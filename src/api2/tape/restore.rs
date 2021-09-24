use std::path::{Path, PathBuf};
use std::ffi::OsStr;
use std::collections::{HashMap, HashSet, BTreeMap};
use std::convert::TryFrom;
use std::io::{Seek, SeekFrom};
use std::sync::Arc;

use anyhow::{bail, format_err, Error};
use serde_json::Value;

use proxmox::{
    api::{
        api,
        RpcEnvironment,
        RpcEnvironmentType,
        Router,
        Permission,
        schema::parse_property_string,
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

use pbs_api_types::{
    Authid, Userid, CryptMode,
    DATASTORE_MAP_ARRAY_SCHEMA, DATASTORE_MAP_LIST_SCHEMA, DRIVE_NAME_SCHEMA,
    UPID_SCHEMA, TAPE_RESTORE_SNAPSHOT_SCHEMA,
    PRIV_DATASTORE_BACKUP, PRIV_DATASTORE_MODIFY, PRIV_TAPE_READ,
};
use pbs_datastore::DataBlob;
use pbs_datastore::backup_info::BackupDir;
use pbs_datastore::dynamic_index::DynamicIndexReader;
use pbs_datastore::fixed_index::FixedIndexReader;
use pbs_datastore::index::IndexFile;
use pbs_datastore::manifest::{archive_type, ArchiveType, BackupManifest, MANIFEST_BLOB_NAME};
use pbs_config::CachedUserInfo;
use pbs_tape::{
    TapeRead, BlockReadError, MediaContentHeader,
    PROXMOX_BACKUP_CONTENT_HEADER_MAGIC_1_0,
};
use pbs_tools::{task_log, task_warn, task::WorkerTaskContext};
use proxmox_rest_server::WorkerTask;

use crate::{
    tools::ParallelHandler,
    backup::DataStore,
    server::lookup_user_email,
    tape::{
        TAPE_STATUS_DIR,
        MediaId,
        MediaSet,
        MediaCatalog,
        MediaSetCatalog,
        Inventory,
        lock_media_set,
        file_formats::{
            PROXMOX_BACKUP_MEDIA_LABEL_MAGIC_1_0,
            PROXMOX_BACKUP_SNAPSHOT_ARCHIVE_MAGIC_1_0,
            PROXMOX_BACKUP_SNAPSHOT_ARCHIVE_MAGIC_1_1,
            PROXMOX_BACKUP_MEDIA_SET_LABEL_MAGIC_1_0,
            PROXMOX_BACKUP_CHUNK_ARCHIVE_MAGIC_1_0,
            PROXMOX_BACKUP_CHUNK_ARCHIVE_MAGIC_1_1,
            PROXMOX_BACKUP_CATALOG_ARCHIVE_MAGIC_1_0,
            ChunkArchiveHeader,
            ChunkArchiveDecoder,
            SnapshotArchiveHeader,
            CatalogArchiveHeader,
        },
        drive::{
            TapeDriver,
            request_and_load_media,
            lock_tape_device,
            set_tape_device_state,
        },
    },
};

const RESTORE_TMP_DIR: &str = "/var/tmp/proxmox-backup";

pub struct DataStoreMap {
    map: HashMap<String, Arc<DataStore>>,
    default: Option<Arc<DataStore>>,
}

impl TryFrom<String> for DataStoreMap {
    type Error = Error;

    fn try_from(value: String) -> Result<Self, Error> {
        let value = parse_property_string(&value, &DATASTORE_MAP_ARRAY_SCHEMA)?;
        let mut mapping: Vec<String> = value
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();

        let mut map = HashMap::new();
        let mut default = None;
        while let Some(mut store) = mapping.pop() {
            if let Some(index) = store.find('=') {
                let mut target = store.split_off(index);
                target.remove(0); // remove '='
                let datastore = DataStore::lookup_datastore(&target)?;
                map.insert(store, datastore);
            } else if default.is_none() {
                default = Some(DataStore::lookup_datastore(&store)?);
            } else {
                bail!("multiple default stores given");
            }
        }

        Ok(Self { map, default })
    }
}

impl DataStoreMap {
    fn used_datastores<'a>(&self) -> HashSet<&str> {
        let mut set = HashSet::new();
        for store in self.map.values() {
            set.insert(store.name());
        }

        if let Some(ref store) = self.default {
            set.insert(store.name());
        }

        set
    }

    fn get_datastore(&self, source: &str) -> Option<Arc<DataStore>> {
        if let Some(store) = self.map.get(source) {
            return Some(Arc::clone(store));
        }
        if let Some(ref store) = self.default {
            return Some(Arc::clone(store));
        }

        return None;
    }
}

fn check_datastore_privs(
    user_info: &CachedUserInfo,
    store: &str,
    auth_id: &Authid,
    owner: &Option<Authid>,
) -> Result<(), Error> {
    let privs = user_info.lookup_privs(&auth_id, &["datastore", &store]);
    if (privs & PRIV_DATASTORE_BACKUP) == 0 {
        bail!("no permissions on /datastore/{}", store);
    }

    if let Some(ref owner) = owner {
        let correct_owner = owner == auth_id
            || (owner.is_token() && !auth_id.is_token() && owner.user() == auth_id.user());

        // same permission as changing ownership after syncing
        if !correct_owner && privs & PRIV_DATASTORE_MODIFY == 0 {
            bail!("no permission to restore as '{}'", owner);
        }
    }

    Ok(())
}

pub const ROUTER: Router = Router::new().post(&API_METHOD_RESTORE);

#[api(
   input: {
        properties: {
            store: {
                schema: DATASTORE_MAP_LIST_SCHEMA,
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
            "snapshots": {
                description: "List of snapshots.",
                type: Array,
                optional: true,
                items: {
                    schema: TAPE_RESTORE_SNAPSHOT_SCHEMA,
                },
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
    snapshots: Option<Vec<String>>,
    owner: Option<Authid>,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let user_info = CachedUserInfo::new()?;

    let store_map = DataStoreMap::try_from(store)
        .map_err(|err| format_err!("cannot parse store mapping: {}", err))?;
    let used_datastores = store_map.used_datastores();
    if used_datastores.len() == 0 {
        bail!("no datastores given");
    }

    for store in used_datastores.iter() {
        check_datastore_privs(&user_info, &store, &auth_id, &owner)?;
    }

    let privs = user_info.lookup_privs(&auth_id, &["tape", "drive", &drive]);
    if (privs & PRIV_TAPE_READ) == 0 {
        bail!("no permissions on /tape/drive/{}", drive);
    }

    let media_set_uuid = media_set.parse()?;

    let status_path = Path::new(TAPE_STATUS_DIR);

    let _lock = lock_media_set(status_path, &media_set_uuid, None)?;

    let inventory = Inventory::load(status_path)?;

    let pool = inventory.lookup_media_set_pool(&media_set_uuid)?;

    let privs = user_info.lookup_privs(&auth_id, &["tape", "pool", &pool]);
    if (privs & PRIV_TAPE_READ) == 0 {
        bail!("no permissions on /tape/pool/{}", pool);
    }

    let (drive_config, _digest) = pbs_config::drive::config()?;

    // early check/lock before starting worker
    let drive_lock = lock_tape_device(&drive_config, &drive)?;

    let to_stdout = rpcenv.env_type() == RpcEnvironmentType::CLI;

    let taskid = used_datastores
        .iter()
        .map(|s| s.to_string())
        .collect::<Vec<String>>()
        .join(", ");

    let upid_str = WorkerTask::new_thread(
        "tape-restore",
        Some(taskid),
        auth_id.to_string(),
        to_stdout,
        move |worker| {
            let _drive_lock = drive_lock; // keep lock guard

            set_tape_device_state(&drive, &worker.upid().to_string())?;

            let restore_owner = owner.as_ref().unwrap_or(&auth_id);

            let email = notify_user
                .as_ref()
                .and_then(|userid| lookup_user_email(userid))
                .or_else(|| lookup_user_email(&auth_id.clone().into()));

            task_log!(worker, "Mediaset '{}'", media_set);
            task_log!(worker, "Pool: {}", pool);

            let res = if let Some(snapshots) = snapshots {
                restore_list_worker(
                    worker.clone(),
                    snapshots,
                    inventory,
                    media_set_uuid,
                    drive_config,
                    &drive,
                    store_map,
                    restore_owner,
                    email,
                )
            } else {
                restore_full_worker(
                    worker.clone(),
                    inventory,
                    media_set_uuid,
                    drive_config,
                    &drive,
                    store_map,
                    restore_owner,
                    email,
                )
            };

            if res.is_ok() {
                task_log!(worker, "Restore mediaset '{}' done", media_set);
            }

            if let Err(err) = set_tape_device_state(&drive, "") {
                task_log!(
                    worker,
                    "could not unset drive state for {}: {}",
                    drive,
                    err
                );
            }

            res
        }
    )?;

    Ok(upid_str.into())
}

fn restore_full_worker(
    worker: Arc<WorkerTask>,
    inventory: Inventory,
    media_set_uuid: Uuid,
    drive_config: SectionConfigData,
    drive_name: &str,
    store_map: DataStoreMap,
    restore_owner: &Authid,
    email: Option<String>,
) -> Result<(), Error> {
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

    if let Some(fingerprint) = encryption_key_fingerprint {
        task_log!(worker, "Encryption key fingerprint: {}", fingerprint);
    }

    task_log!(
        worker,
        "Datastore(s): {}",
        store_map
        .used_datastores()
        .into_iter()
        .map(String::from)
        .collect::<Vec<String>>()
        .join(", "),
    );

    task_log!(worker, "Drive: {}", drive_name);
    task_log!(
        worker,
        "Required media list: {}",
        media_id_list.iter()
        .map(|media_id| media_id.label.label_text.as_str())
        .collect::<Vec<&str>>()
        .join(";")
    );

    let mut datastore_locks = Vec::new();
    for store_name in store_map.used_datastores() {
        // explicit create shared lock to prevent GC on newly created chunks
        if let Some(store) = store_map.get_datastore(store_name) {
            let shared_store_lock = store.try_shared_chunk_store_lock()?;
            datastore_locks.push(shared_store_lock);
        }
    }

    let mut checked_chunks_map = HashMap::new();

    for media_id in media_id_list.iter() {
        request_and_restore_media(
            worker.clone(),
            media_id,
            &drive_config,
            drive_name,
            &store_map,
            &mut checked_chunks_map,
            restore_owner,
            &email,
        )?;
    }

    Ok(())
}

fn restore_list_worker(
    worker: Arc<WorkerTask>,
    snapshots: Vec<String>,
    inventory: Inventory,
    media_set_uuid: Uuid,
    drive_config: SectionConfigData,
    drive_name: &str,
    store_map: DataStoreMap,
    restore_owner: &Authid,
    email: Option<String>,
) -> Result<(), Error> {
    let base_path: PathBuf = format!("{}/{}", RESTORE_TMP_DIR, media_set_uuid).into();
    std::fs::create_dir_all(&base_path)?;

    let catalog = get_media_set_catalog(&inventory, &media_set_uuid)?;

    let mut datastore_locks = Vec::new();
    let mut snapshot_file_hash: BTreeMap<Uuid, Vec<u64>> = BTreeMap::new();
    let mut snapshot_locks = HashMap::new();

    let res = proxmox::try_block!({
        // assemble snapshot files/locks
        for store_snapshot in snapshots.iter() {
            let mut split = store_snapshot.splitn(2, ':');
            let source_datastore = split
                .next()
                .ok_or_else(|| format_err!("invalid snapshot: {}", store_snapshot))?;
            let snapshot = split
                .next()
                .ok_or_else(|| format_err!("invalid snapshot:{}", store_snapshot))?;
            let backup_dir: BackupDir = snapshot.parse()?;

            let datastore = store_map.get_datastore(source_datastore).ok_or_else(|| {
                format_err!(
                    "could not find mapping for source datastore: {}",
                    source_datastore
                )
            })?;

            let (owner, _group_lock) =
                datastore.create_locked_backup_group(backup_dir.group(), &restore_owner)?;
            if restore_owner != &owner {
                // only the owner is allowed to create additional snapshots
                bail!(
                    "restore '{}' failed - owner check failed ({} != {})",
                    snapshot,
                    restore_owner,
                    owner
                );
            }

            let (media_id, file_num) = if let Some((media_uuid, file_num)) =
                catalog.lookup_snapshot(&source_datastore, &snapshot)
            {
                let media_id = inventory.lookup_media(media_uuid).unwrap();
                (media_id, file_num)
            } else {
                task_warn!(
                    worker,
                    "did not find snapshot '{}' in media set {}",
                    snapshot,
                    media_set_uuid
                );
                continue;
            };

            let (_rel_path, is_new, snap_lock) = datastore.create_locked_backup_dir(&backup_dir)?;

            if !is_new {
                task_log!(
                    worker,
                    "found snapshot {} on target datastore, skipping...",
                    snapshot
                );
                continue;
            }

            snapshot_locks.insert(store_snapshot.to_string(), snap_lock);

            let shared_store_lock = datastore.try_shared_chunk_store_lock()?;
            datastore_locks.push(shared_store_lock);

            let file_list = snapshot_file_hash
                .entry(media_id.label.uuid.clone())
                .or_insert_with(Vec::new);
            file_list.push(file_num);

            task_log!(
                worker,
                "found snapshot {} on {}: file {}",
                snapshot,
                media_id.label.label_text,
                file_num
            );
        }

        if snapshot_file_hash.is_empty() {
            task_log!(worker, "nothing to restore, skipping remaining phases...");
            return Ok(());
        }

        task_log!(worker, "Phase 1: temporarily restore snapshots to temp dir");
        let mut datastore_chunk_map: HashMap<String, HashSet<[u8; 32]>> = HashMap::new();
        for (media_uuid, file_list) in snapshot_file_hash.iter_mut() {
            let media_id = inventory.lookup_media(media_uuid).unwrap();
            let (drive, info) = request_and_load_media(
                &worker,
                &drive_config,
                &drive_name,
                &media_id.label,
                &email,
            )?;
            file_list.sort_unstable();
            restore_snapshots_to_tmpdir(
                worker.clone(),
                &base_path,
                file_list,
                drive,
                &info,
                &media_set_uuid,
                &mut datastore_chunk_map,
            ).map_err(|err| format_err!("could not restore snapshots to tmpdir: {}", err))?;
        }

        // sorted media_uuid => (sorted file_num => (set of digests)))
        let mut media_file_chunk_map: BTreeMap<Uuid, BTreeMap<u64, HashSet<[u8; 32]>>> = BTreeMap::new();

        for (source_datastore, chunks) in datastore_chunk_map.into_iter() {
            let datastore = store_map.get_datastore(&source_datastore).ok_or_else(|| {
                format_err!(
                    "could not find mapping for source datastore: {}",
                    source_datastore
                )
            })?;
            for digest in chunks.into_iter() {
                // we only want to restore chunks that we do not have yet
                if !datastore.cond_touch_chunk(&digest, false)? {
                    if let Some((uuid, nr)) = catalog.lookup_chunk(&source_datastore, &digest) {
                        let file = media_file_chunk_map.entry(uuid.clone()).or_insert_with(BTreeMap::new);
                        let chunks = file.entry(nr).or_insert_with(HashSet::new);
                        chunks.insert(digest);
                    }
                }
            }
        }

        // we do not need it anymore, saves memory
        drop(catalog);

        if !media_file_chunk_map.is_empty() {
            task_log!(worker, "Phase 2: restore chunks to datastores");
        } else {
            task_log!(worker, "all chunks exist already, skipping phase 2...");
        }

        for (media_uuid, file_chunk_map) in media_file_chunk_map.iter_mut() {
            let media_id = inventory.lookup_media(media_uuid).unwrap();
            let (mut drive, _info) = request_and_load_media(
                &worker,
                &drive_config,
                &drive_name,
                &media_id.label,
                &email,
            )?;
            restore_file_chunk_map(worker.clone(), &mut drive, &store_map, file_chunk_map)?;
        }

        task_log!(
            worker,
            "Phase 3: copy snapshots from temp dir to datastores"
        );
        for (store_snapshot, _lock) in snapshot_locks.into_iter() {
            proxmox::try_block!({
                let mut split = store_snapshot.splitn(2, ':');
                let source_datastore = split
                    .next()
                    .ok_or_else(|| format_err!("invalid snapshot: {}", store_snapshot))?;
                let snapshot = split
                    .next()
                    .ok_or_else(|| format_err!("invalid snapshot:{}", store_snapshot))?;
                let backup_dir: BackupDir = snapshot.parse()?;

                let datastore = store_map
                    .get_datastore(&source_datastore)
                    .ok_or_else(|| format_err!("unexpected source datastore: {}", source_datastore))?;

                let mut tmp_path = base_path.clone();
                tmp_path.push(&source_datastore);
                tmp_path.push(snapshot);

                let path = datastore.snapshot_path(&backup_dir);

                for entry in std::fs::read_dir(tmp_path)? {
                    let entry = entry?;
                    let mut new_path = path.clone();
                    new_path.push(entry.file_name());
                    std::fs::copy(entry.path(), new_path)?;
                }
                task_log!(worker, "Restore snapshot '{}' done", snapshot);
                Ok(())
            }).map_err(|err: Error| format_err!("could not copy {}: {}", store_snapshot, err))?;
        }
        Ok(())
    });

    if res.is_err() {
        task_warn!(worker, "Error during restore, partially restored snapshots will NOT be cleaned up");
    }

    match std::fs::remove_dir_all(&base_path) {
        Ok(()) => {}
        Err(err) => task_warn!(worker, "error cleaning up: {}", err),
    }

    res
}

fn get_media_set_catalog(
    inventory: &Inventory,
    media_set_uuid: &Uuid,
) -> Result<MediaSetCatalog, Error> {
    let status_path = Path::new(TAPE_STATUS_DIR);

    let members = inventory.compute_media_set_members(media_set_uuid)?;
    let media_list = members.media_list();
    let mut catalog = MediaSetCatalog::new();

    for (seq_nr, media_uuid) in media_list.iter().enumerate() {
        match media_uuid {
            None => {
                bail!(
                    "media set {} is incomplete (missing member {}).",
                    media_set_uuid,
                    seq_nr
                );
            }
            Some(media_uuid) => {
                let media_id = inventory.lookup_media(media_uuid).unwrap();
                let media_catalog = MediaCatalog::open(status_path, &media_id, false, false)?;
                catalog.append_catalog(media_catalog)?;
            }
        }
    }

    Ok(catalog)
}

fn restore_snapshots_to_tmpdir(
    worker: Arc<WorkerTask>,
    path: &PathBuf,
    file_list: &[u64],
    mut drive: Box<dyn TapeDriver>,
    media_id: &MediaId,
    media_set_uuid: &Uuid,
    chunks_list: &mut HashMap<String, HashSet<[u8; 32]>>,
) -> Result<(), Error> {
    match media_id.media_set_label {
        None => {
            bail!(
                "missing media set label on media {} ({})",
                media_id.label.label_text,
                media_id.label.uuid
            );
        }
        Some(ref set) => {
            if set.uuid != *media_set_uuid {
                bail!(
                    "wrong media set label on media {} ({} != {})",
                    media_id.label.label_text,
                    media_id.label.uuid,
                    media_set_uuid
                );
            }
            let encrypt_fingerprint = set.encryption_key_fingerprint.clone().map(|fp| {
                task_log!(worker, "Encryption key fingerprint: {}", fp);
                (fp, set.uuid.clone())
            });

            drive.set_encryption(encrypt_fingerprint)?;
        }
    }

    for file_num in file_list {
        let current_file_number = drive.current_file_number()?;
        if current_file_number != *file_num {
            task_log!(worker, "was at file {}, moving to {}", current_file_number, file_num);
            drive.move_to_file(*file_num)?;
            let current_file_number = drive.current_file_number()?;
            task_log!(worker, "now at file {}", current_file_number);
        }
        let mut reader = drive.read_next_file()?;

        let header: MediaContentHeader = unsafe { reader.read_le_value()? };
        if header.magic != PROXMOX_BACKUP_CONTENT_HEADER_MAGIC_1_0 {
            bail!("missing MediaContentHeader");
        }

        match header.content_magic {
            PROXMOX_BACKUP_SNAPSHOT_ARCHIVE_MAGIC_1_1 => {
                let header_data = reader.read_exact_allocated(header.size as usize)?;

                let archive_header: SnapshotArchiveHeader = serde_json::from_slice(&header_data)
                    .map_err(|err| {
                        format_err!("unable to parse snapshot archive header - {}", err)
                    })?;

                let source_datastore = archive_header.store;
                let snapshot = archive_header.snapshot;

                task_log!(
                    worker,
                    "File {}: snapshot archive {}:{}",
                    file_num,
                    source_datastore,
                    snapshot
                );

                let mut decoder = pxar::decoder::sync::Decoder::from_std(reader)?;

                let mut tmp_path = path.clone();
                tmp_path.push(&source_datastore);
                tmp_path.push(snapshot);
                std::fs::create_dir_all(&tmp_path)?;

                let chunks = chunks_list
                    .entry(source_datastore)
                    .or_insert_with(HashSet::new);
                let manifest = try_restore_snapshot_archive(worker.clone(), &mut decoder, &tmp_path)?;
                for item in manifest.files() {
                    let mut archive_path = tmp_path.to_owned();
                    archive_path.push(&item.filename);

                    let index: Box<dyn IndexFile> = match archive_type(&item.filename)? {
                        ArchiveType::DynamicIndex => {
                            Box::new(DynamicIndexReader::open(&archive_path)?)
                        }
                        ArchiveType::FixedIndex => {
                            Box::new(FixedIndexReader::open(&archive_path)?)
                        }
                        ArchiveType::Blob => continue,
                    };
                    for i in 0..index.index_count() {
                        if let Some(digest) = index.index_digest(i) {
                            chunks.insert(*digest);
                        }
                    }
                }
            }
            other => bail!("unexpected file type: {:?}", other),
        }
    }

    Ok(())
}

fn restore_file_chunk_map(
    worker: Arc<WorkerTask>,
    drive: &mut Box<dyn TapeDriver>,
    store_map: &DataStoreMap,
    file_chunk_map: &mut BTreeMap<u64, HashSet<[u8; 32]>>,
) -> Result<(), Error> {
    for (nr, chunk_map) in file_chunk_map.iter_mut() {
        let current_file_number = drive.current_file_number()?;
        if current_file_number != *nr {
            task_log!(worker, "was at file {}, moving to {}", current_file_number, nr);
            drive.move_to_file(*nr)?;
            let current_file_number = drive.current_file_number()?;
            task_log!(worker, "now at file {}", current_file_number);
        }
        let mut reader = drive.read_next_file()?;
        let header: MediaContentHeader = unsafe { reader.read_le_value()? };
        if header.magic != PROXMOX_BACKUP_CONTENT_HEADER_MAGIC_1_0 {
            bail!("missing MediaContentHeader");
        }

        match header.content_magic {
            PROXMOX_BACKUP_CHUNK_ARCHIVE_MAGIC_1_1 => {
                let header_data = reader.read_exact_allocated(header.size as usize)?;

                let archive_header: ChunkArchiveHeader = serde_json::from_slice(&header_data)
                    .map_err(|err| format_err!("unable to parse chunk archive header - {}", err))?;

                let source_datastore = archive_header.store;

                task_log!(
                    worker,
                    "File {}: chunk archive for datastore '{}'",
                    nr,
                    source_datastore
                );

                let datastore = store_map.get_datastore(&source_datastore).ok_or_else(|| {
                    format_err!("unexpected chunk archive for store: {}", source_datastore)
                })?;

                let count = restore_partial_chunk_archive(worker.clone(), reader, datastore.clone(), chunk_map)?;
                task_log!(worker, "restored {} chunks", count);
            }
            _ => bail!("unexpected content magic {:?}", header.content_magic),
        }
    }

    Ok(())
}

fn restore_partial_chunk_archive<'a>(
    worker: Arc<WorkerTask>,
    reader: Box<dyn 'a + TapeRead>,
    datastore: Arc<DataStore>,
    chunk_list: &mut HashSet<[u8; 32]>,
) -> Result<usize, Error> {
    let mut decoder = ChunkArchiveDecoder::new(reader);

    let mut count = 0;

    let start_time = std::time::SystemTime::now();
    let bytes = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let bytes2 = bytes.clone();

    let writer_pool = ParallelHandler::new(
        "tape restore chunk writer",
        4,
        move |(chunk, digest): (DataBlob, [u8; 32])| {
            if !datastore.cond_touch_chunk(&digest, false)? {
                bytes2.fetch_add(chunk.raw_size(), std::sync::atomic::Ordering::SeqCst);
                chunk.verify_crc()?;
                if chunk.crypt_mode()? == CryptMode::None {
                    chunk.decode(None, Some(&digest))?; // verify digest
                }

                datastore.insert_chunk(&chunk, &digest)?;
            }
            Ok(())
        },
    );

    let verify_and_write_channel = writer_pool.channel();

    loop {
        let (digest, blob) = match decoder.next_chunk()? {
            Some((digest, blob)) => (digest, blob),
            None => break,
        };

        worker.check_abort()?;

        if chunk_list.remove(&digest) {
            verify_and_write_channel.send((blob, digest.clone()))?;
            count += 1;
        }

        if chunk_list.is_empty() {
            break;
        }
    }

    drop(verify_and_write_channel);

    writer_pool.complete()?;

    let elapsed = start_time.elapsed()?.as_secs_f64();

    let bytes = bytes.load(std::sync::atomic::Ordering::SeqCst);

    task_log!(
        worker,
        "restored {} bytes ({:.2} MB/s)",
        bytes,
        (bytes as f64) / (1_000_000.0 * elapsed)
    );

    Ok(count)
}


/// Request and restore complete media without using existing catalog (create catalog instead)
pub fn request_and_restore_media(
    worker: Arc<WorkerTask>,
    media_id: &MediaId,
    drive_config: &SectionConfigData,
    drive_name: &str,
    store_map: &DataStoreMap,
    checked_chunks_map: &mut HashMap<String, HashSet<[u8;32]>>,
    restore_owner: &Authid,
    email: &Option<String>,
) -> Result<(), Error> {
    let media_set_uuid = match media_id.media_set_label {
        None => bail!("restore_media: no media set - internal error"),
        Some(ref set) => &set.uuid,
    };

    let (mut drive, info) = request_and_load_media(&worker, &drive_config, &drive_name, &media_id.label, email)?;

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

    restore_media(
        worker,
        &mut drive,
        &info,
        Some((&store_map, restore_owner)),
        checked_chunks_map,
        false,
    )
}

/// Restore complete media content and catalog
///
/// Only create the catalog if target is None.
pub fn restore_media(
    worker: Arc<WorkerTask>,
    drive: &mut Box<dyn TapeDriver>,
    media_id: &MediaId,
    target: Option<(&DataStoreMap, &Authid)>,
    checked_chunks_map: &mut HashMap<String, HashSet<[u8;32]>>,
    verbose: bool,
) ->  Result<(), Error> {

    let status_path = Path::new(TAPE_STATUS_DIR);
    let mut catalog = MediaCatalog::create_temporary_database(status_path, media_id, false)?;

    loop {
        let current_file_number = drive.current_file_number()?;
        let reader = match drive.read_next_file() {
            Err(BlockReadError::EndOfFile) => {
                task_log!(worker, "skip unexpected filemark at pos {}", current_file_number);
                continue;
            }
            Err(BlockReadError::EndOfStream) => {
                task_log!(worker, "detected EOT after {} files", current_file_number);
                break;
            }
            Err(BlockReadError::Error(err)) => {
                return Err(err.into());
            }
            Ok(reader) => reader,
        };

        restore_archive(worker.clone(), reader, current_file_number, target, &mut catalog, checked_chunks_map, verbose)?;
    }

    catalog.commit()?;

    MediaCatalog::finish_temporary_database(status_path, &media_id.label.uuid, true)?;

    Ok(())
}

fn restore_archive<'a>(
    worker: Arc<WorkerTask>,
    mut reader: Box<dyn 'a + TapeRead>,
    current_file_number: u64,
    target: Option<(&DataStoreMap, &Authid)>,
    catalog: &mut MediaCatalog,
    checked_chunks_map: &mut HashMap<String, HashSet<[u8;32]>>,
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
            bail!("unexpected snapshot archive version (v1.0)");
        }
        PROXMOX_BACKUP_SNAPSHOT_ARCHIVE_MAGIC_1_1 => {
            let header_data = reader.read_exact_allocated(header.size as usize)?;

            let archive_header: SnapshotArchiveHeader = serde_json::from_slice(&header_data)
                .map_err(|err| format_err!("unable to parse snapshot archive header - {}", err))?;

            let datastore_name = archive_header.store;
            let snapshot = archive_header.snapshot;

            task_log!(worker, "File {}: snapshot archive {}:{}", current_file_number, datastore_name, snapshot);

            let backup_dir: BackupDir = snapshot.parse()?;

            if let Some((store_map, authid)) = target.as_ref() {
                if let Some(datastore) = store_map.get_datastore(&datastore_name) {
                    let (owner, _group_lock) =
                        datastore.create_locked_backup_group(backup_dir.group(), authid)?;
                    if *authid != &owner {
                        // only the owner is allowed to create additional snapshots
                        bail!(
                            "restore '{}' failed - owner check failed ({} != {})",
                            snapshot,
                            authid,
                            owner
                        );
                    }

                    let (rel_path, is_new, _snap_lock) =
                        datastore.create_locked_backup_dir(&backup_dir)?;
                    let mut path = datastore.base_path();
                    path.push(rel_path);

                    if is_new {
                        task_log!(worker, "restore snapshot {}", backup_dir);

                        match restore_snapshot_archive(worker.clone(), reader, &path) {
                            Err(err) => {
                                std::fs::remove_dir_all(&path)?;
                                bail!("restore snapshot {} failed - {}", backup_dir, err);
                            }
                            Ok(false) => {
                                std::fs::remove_dir_all(&path)?;
                                task_log!(worker, "skip incomplete snapshot {}", backup_dir);
                            }
                            Ok(true) => {
                                catalog.register_snapshot(
                                    Uuid::from(header.uuid),
                                    current_file_number,
                                    &datastore_name,
                                    &snapshot,
                                )?;
                                catalog.commit_if_large()?;
                            }
                        }
                        return Ok(());
                    }
                } else {
                    task_log!(worker, "skipping...");
                }
            }

            reader.skip_data()?; // read all data
            if let Ok(false) = reader.is_incomplete() {
                catalog.register_snapshot(Uuid::from(header.uuid), current_file_number, &datastore_name, &snapshot)?;
                catalog.commit_if_large()?;
            }
        }
        PROXMOX_BACKUP_CHUNK_ARCHIVE_MAGIC_1_0 => {
            bail!("unexpected chunk archive version (v1.0)");
        }
        PROXMOX_BACKUP_CHUNK_ARCHIVE_MAGIC_1_1 => {
            let header_data = reader.read_exact_allocated(header.size as usize)?;

            let archive_header: ChunkArchiveHeader = serde_json::from_slice(&header_data)
                .map_err(|err| format_err!("unable to parse chunk archive header - {}", err))?;

            let source_datastore = archive_header.store;

            task_log!(worker, "File {}: chunk archive for datastore '{}'", current_file_number, source_datastore);
            let datastore = target
                .as_ref()
                .and_then(|t| t.0.get_datastore(&source_datastore));

            if datastore.is_some() || target.is_none() {
                let checked_chunks = checked_chunks_map
                    .entry(datastore.as_ref().map(|d| d.name()).unwrap_or("_unused_").to_string())
                    .or_insert(HashSet::new());

                let chunks = if let Some(datastore) = datastore {
                    restore_chunk_archive(worker.clone(), reader, datastore, checked_chunks, verbose)?
                } else {
                    scan_chunk_archive(worker.clone(), reader, verbose)?
                };

                if let Some(chunks) = chunks {
                    catalog.register_chunk_archive(
                        Uuid::from(header.uuid),
                        current_file_number,
                        &source_datastore,
                        &chunks[..],
                    )?;
                    task_log!(worker, "register {} chunks", chunks.len());
                    catalog.commit_if_large()?;
                }
                return Ok(());
            } else if target.is_some() {
                task_log!(worker, "skipping...");
            }

            reader.skip_data()?; // read all data
        }
        PROXMOX_BACKUP_CATALOG_ARCHIVE_MAGIC_1_0 => {
            let header_data = reader.read_exact_allocated(header.size as usize)?;

            let archive_header: CatalogArchiveHeader = serde_json::from_slice(&header_data)
                .map_err(|err| format_err!("unable to parse catalog archive header - {}", err))?;

            task_log!(worker, "File {}: skip catalog '{}'", current_file_number, archive_header.uuid);

            reader.skip_data()?; // read all data
        }
         _ =>  bail!("unknown content magic {:?}", header.content_magic),
    }

    Ok(())
}

// Read chunk archive without restoring data - just record contained chunks
fn scan_chunk_archive<'a>(
    worker: Arc<WorkerTask>,
    reader: Box<dyn 'a + TapeRead>,
    verbose: bool,
) -> Result<Option<Vec<[u8;32]>>, Error> {

    let mut chunks = Vec::new();

    let mut decoder = ChunkArchiveDecoder::new(reader);

    loop {
        let digest = match decoder.next_chunk() {
            Ok(Some((digest, _blob))) => digest,
            Ok(None) => break,
            Err(err) => {
                let reader = decoder.reader();

                // check if this stream is marked incomplete
                if let Ok(true) = reader.is_incomplete() {
                    return Ok(Some(chunks));
                }

                // check if this is an aborted stream without end marker
                if let Ok(false) = reader.has_end_marker() {
                    task_log!(worker, "missing stream end marker");
                    return Ok(None);
                }

                // else the archive is corrupt
                return Err(err);
            }
        };

        worker.check_abort()?;

        if verbose {
            task_log!(worker, "Found chunk: {}", proxmox::tools::digest_to_hex(&digest));
        }

        chunks.push(digest);
    }

    Ok(Some(chunks))
}

fn restore_chunk_archive<'a>(
    worker: Arc<WorkerTask>,
    reader: Box<dyn 'a + TapeRead>,
    datastore: Arc<DataStore>,
    checked_chunks: &mut HashSet<[u8;32]>,
    verbose: bool,
) -> Result<Option<Vec<[u8;32]>>, Error> {

    let mut chunks = Vec::new();

    let mut decoder = ChunkArchiveDecoder::new(reader);

    let datastore2 = datastore.clone();
    let start_time = std::time::SystemTime::now();
    let bytes = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let bytes2 = bytes.clone();

    let worker2 = worker.clone();

    let writer_pool = ParallelHandler::new(
        "tape restore chunk writer",
        4,
        move |(chunk, digest): (DataBlob, [u8; 32])| {
            let chunk_exists = datastore2.cond_touch_chunk(&digest, false)?;
            if !chunk_exists {
                if verbose {
                    task_log!(worker2, "Insert chunk: {}", proxmox::tools::digest_to_hex(&digest));
                }
                bytes2.fetch_add(chunk.raw_size(), std::sync::atomic::Ordering::SeqCst);
                // println!("verify and write {}", proxmox::tools::digest_to_hex(&digest));
                chunk.verify_crc()?;
                if chunk.crypt_mode()? == CryptMode::None {
                    chunk.decode(None, Some(&digest))?; // verify digest
                }

                datastore2.insert_chunk(&chunk, &digest)?;
            } else if verbose {
                task_log!(worker2, "Found existing chunk: {}", proxmox::tools::digest_to_hex(&digest));
            }
            Ok(())
        },
    );

    let verify_and_write_channel = writer_pool.channel();


    loop {
        let (digest, blob) = match decoder.next_chunk() {
            Ok(Some((digest, blob))) => (digest, blob),
            Ok(None) => break,
            Err(err) => {
                let reader = decoder.reader();

                // check if this stream is marked incomplete
                if let Ok(true) = reader.is_incomplete() {
                    return Ok(Some(chunks));
                }

                // check if this is an aborted stream without end marker
                if let Ok(false) = reader.has_end_marker() {
                    task_log!(worker, "missing stream end marker");
                    return Ok(None);
                }

                // else the archive is corrupt
                return Err(err);
            }
        };

        worker.check_abort()?;

        if !checked_chunks.contains(&digest) {
            verify_and_write_channel.send((blob, digest.clone()))?;
            checked_chunks.insert(digest.clone());
        }
        chunks.push(digest);
    }

    drop(verify_and_write_channel);

    writer_pool.complete()?;

    let elapsed = start_time.elapsed()?.as_secs_f64();

    let bytes = bytes.load(std::sync::atomic::Ordering::SeqCst);

    task_log!(
        worker,
        "restored {} bytes ({:.2} MB/s)",
        bytes,
        (bytes as f64) / (1_000_000.0 * elapsed)
    );

    Ok(Some(chunks))
}

fn restore_snapshot_archive<'a>(
    worker: Arc<WorkerTask>,
    reader: Box<dyn 'a + TapeRead>,
    snapshot_path: &Path,
) -> Result<bool, Error> {

    let mut decoder = pxar::decoder::sync::Decoder::from_std(reader)?;
    match try_restore_snapshot_archive(worker, &mut decoder, snapshot_path) {
        Ok(_) => Ok(true),
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
    worker: Arc<WorkerTask>,
    decoder: &mut pxar::decoder::sync::Decoder<R>,
    snapshot_path: &Path,
) -> Result<BackupManifest, Error> {

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
            let mut old_manifest = BackupManifest::try_from(blob)?;

            // Remove verify_state to indicate that this snapshot is not verified
            old_manifest.unprotected
                .as_object_mut()
                .map(|m| m.remove("verify_state"));

            let old_manifest = serde_json::to_string_pretty(&old_manifest)?;
            let blob = DataBlob::encode(old_manifest.as_bytes(), None, true)?;

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

    // Do not verify anything here, because this would be to slow (causes tape stops).

    // commit manifest
    let mut manifest_path = snapshot_path.to_owned();
    manifest_path.push(MANIFEST_BLOB_NAME);
    let mut tmp_manifest_path = manifest_path.clone();
    tmp_manifest_path.set_extension("tmp");

    if let Err(err) = std::fs::rename(&tmp_manifest_path, &manifest_path) {
        bail!("Atomic rename manifest {:?} failed - {}", manifest_path, err);
    }

    Ok(manifest)
}

/// Try to restore media catalogs (form catalog_archives)
pub fn fast_catalog_restore(
    worker: &WorkerTask,
    drive: &mut Box<dyn TapeDriver>,
    media_set: &MediaSet,
    uuid: &Uuid, // current media Uuid
) ->  Result<bool, Error> {

    let status_path = Path::new(TAPE_STATUS_DIR);

    let current_file_number = drive.current_file_number()?;
    if current_file_number != 2 {
        bail!("fast_catalog_restore: wrong media position - internal error");
    }

    let mut found_catalog = false;

    let mut moved_to_eom = false;

    loop {
        let current_file_number = drive.current_file_number()?;

        { // limit reader scope
            let mut reader = match drive.read_next_file() {
                Err(BlockReadError::EndOfFile) => {
                    task_log!(worker, "skip unexpected filemark at pos {}", current_file_number);
                    continue;
                }
                Err(BlockReadError::EndOfStream) => {
                    task_log!(worker, "detected EOT after {} files", current_file_number);
                    break;
                }
                Err(BlockReadError::Error(err)) => {
                    return Err(err.into());
                }
                Ok(reader) => reader,
            };

            let header: MediaContentHeader = unsafe { reader.read_le_value()? };
            if header.magic != PROXMOX_BACKUP_CONTENT_HEADER_MAGIC_1_0 {
                bail!("missing MediaContentHeader");
            }

            if header.content_magic == PROXMOX_BACKUP_CATALOG_ARCHIVE_MAGIC_1_0 {
                task_log!(worker, "found catalog at pos {}", current_file_number);

                let header_data = reader.read_exact_allocated(header.size as usize)?;

                let archive_header: CatalogArchiveHeader = serde_json::from_slice(&header_data)
                    .map_err(|err| format_err!("unable to parse catalog archive header - {}", err))?;

                if &archive_header.media_set_uuid != media_set.uuid() {
                    task_log!(worker, "skipping unrelated catalog at pos {}", current_file_number);
                    reader.skip_data()?; // read all data
                    continue;
                }

                let catalog_uuid = &archive_header.uuid;

                let wanted = media_set
                    .media_list()
                    .iter()
                    .find(|e| {
                        match e {
                            None => false,
                            Some(uuid) => uuid == catalog_uuid,
                        }
                    })
                    .is_some();

                if !wanted {
                    task_log!(worker, "skip catalog because media '{}' not inventarized", catalog_uuid);
                    reader.skip_data()?; // read all data
                    continue;
                }

                if catalog_uuid == uuid {
                    // always restore and overwrite catalog
                } else {
                    // only restore if catalog does not exist
                    if MediaCatalog::exists(status_path, catalog_uuid) {
                        task_log!(worker, "catalog for media '{}' already exists", catalog_uuid);
                        reader.skip_data()?; // read all data
                        continue;
                    }
                }

                let mut file = MediaCatalog::create_temporary_database_file(status_path, catalog_uuid)?;

                std::io::copy(&mut reader, &mut file)?;

                file.seek(SeekFrom::Start(0))?;

                match MediaCatalog::parse_catalog_header(&mut file)? {
                    (true, Some(media_uuid), Some(media_set_uuid)) => {
                        if &media_uuid != catalog_uuid {
                            task_log!(worker, "catalog uuid missmatch at pos {}", current_file_number);
                            continue;
                        }
                        if media_set_uuid != archive_header.media_set_uuid {
                            task_log!(worker, "catalog media_set missmatch at pos {}", current_file_number);
                            continue;
                        }

                        MediaCatalog::finish_temporary_database(status_path, &media_uuid, true)?;

                        if catalog_uuid == uuid {
                            task_log!(worker, "successfully restored catalog");
                            found_catalog = true
                        } else {
                            task_log!(worker, "successfully restored related catalog {}", media_uuid);
                        }
                    }
                    _ => {
                        task_warn!(worker, "got incomplete catalog header - skip file");
                        continue;
                    }
                }

                continue;
            }
        }

        if moved_to_eom {
            break; // already done - stop
        }
        moved_to_eom = true;

        task_log!(worker, "searching for catalog at EOT (moving to EOT)");
        drive.move_to_last_file()?;

        let new_file_number = drive.current_file_number()?;

        if new_file_number < (current_file_number + 1) {
            break; // no new content - stop
        }
    }

    Ok(found_catalog)
}
