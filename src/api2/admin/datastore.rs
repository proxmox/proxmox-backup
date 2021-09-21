//! Datastore Management

use std::collections::HashSet;
use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

use anyhow::{bail, format_err, Error};
use futures::*;
use hyper::http::request::Parts;
use hyper::{header, Body, Response, StatusCode};
use serde_json::{json, Value};
use tokio_stream::wrappers::ReceiverStream;

use proxmox::api::{
    api, ApiResponseFuture, ApiHandler, ApiMethod, Router,
    RpcEnvironment, RpcEnvironmentType, Permission
};
use proxmox::api::router::SubdirMap;
use proxmox::api::schema::*;
use proxmox::tools::fs::{
    file_read_firstline, file_read_optional_string, replace_file, CreateOptions,
};
use proxmox::{http_err, identity, list_subdirs_api_method, sortable};

use pxar::accessor::aio::Accessor;
use pxar::EntryKind;

use pbs_api_types::{ Authid, BackupContent, Counts, CryptMode,
    DataStoreListItem, GarbageCollectionStatus, GroupListItem,
    SnapshotListItem, SnapshotVerifyState, PruneOptions,
    DataStoreStatus, RRDMode, RRDTimeFrameResolution,
    BACKUP_ARCHIVE_NAME_SCHEMA, BACKUP_ID_SCHEMA, BACKUP_TIME_SCHEMA,
    BACKUP_TYPE_SCHEMA, DATASTORE_SCHEMA,
    IGNORE_VERIFIED_BACKUPS_SCHEMA, UPID_SCHEMA,
    VERIFICATION_OUTDATED_AFTER_SCHEMA, PRIV_DATASTORE_AUDIT,
    PRIV_DATASTORE_MODIFY, PRIV_DATASTORE_READ, PRIV_DATASTORE_PRUNE,
    PRIV_DATASTORE_BACKUP, PRIV_DATASTORE_VERIFY,

};
use pbs_client::pxar::create_zip;
use pbs_datastore::{BackupDir, BackupGroup, StoreProgress, CATALOG_NAME};
use pbs_datastore::backup_info::BackupInfo;
use pbs_datastore::cached_chunk_reader::CachedChunkReader;
use pbs_datastore::catalog::{ArchiveEntry, CatalogReader};
use pbs_datastore::data_blob::DataBlob;
use pbs_datastore::data_blob_reader::DataBlobReader;
use pbs_datastore::dynamic_index::{BufferedDynamicReader, DynamicIndexReader, LocalDynamicReadAt};
use pbs_datastore::fixed_index::{FixedIndexReader};
use pbs_datastore::index::IndexFile;
use pbs_datastore::manifest::{BackupManifest, CLIENT_LOG_BLOB_NAME, MANIFEST_BLOB_NAME};
use pbs_datastore::prune::compute_prune_info;
use pbs_tools::blocking::WrappedReaderStream;
use pbs_tools::stream::{AsyncReaderStream, AsyncChannelWriter};
use pbs_tools::json::{required_integer_param, required_string_param};
use pbs_config::CachedUserInfo;
use proxmox_rest_server::formatter;

use crate::api2::node::rrd::create_value_from_rrd;
use crate::backup::{
    check_backup_owner, verify_all_backups, verify_backup_group, verify_backup_dir, verify_filter,
    DataStore, LocalChunkReader,
};

use crate::server::{jobstate::Job, WorkerTask};


const GROUP_NOTES_FILE_NAME: &str = "notes";

fn get_group_note_path(store: &DataStore, group: &BackupGroup) -> PathBuf {
    let mut note_path = store.base_path();
    note_path.push(group.group_path());
    note_path.push(GROUP_NOTES_FILE_NAME);
    note_path
}

fn check_priv_or_backup_owner(
    store: &DataStore,
    group: &BackupGroup,
    auth_id: &Authid,
    required_privs: u64,
) -> Result<(), Error> {
    let user_info = CachedUserInfo::new()?;
    let privs = user_info.lookup_privs(&auth_id, &["datastore", store.name()]);

    if privs & required_privs == 0 {
        let owner = store.get_owner(group)?;
        check_backup_owner(&owner, auth_id)?;
    }
    Ok(())
}

fn read_backup_index(
    store: &DataStore,
    backup_dir: &BackupDir,
) -> Result<(BackupManifest, Vec<BackupContent>), Error> {

    let (manifest, index_size) = store.load_manifest(backup_dir)?;

    let mut result = Vec::new();
    for item in manifest.files() {
        result.push(BackupContent {
            filename: item.filename.clone(),
            crypt_mode: Some(item.crypt_mode),
            size: Some(item.size),
        });
    }

    result.push(BackupContent {
        filename: MANIFEST_BLOB_NAME.to_string(),
        crypt_mode: match manifest.signature {
            Some(_) => Some(CryptMode::SignOnly),
            None => Some(CryptMode::None),
        },
        size: Some(index_size),
    });

    Ok((manifest, result))
}

fn get_all_snapshot_files(
    store: &DataStore,
    info: &BackupInfo,
) -> Result<(BackupManifest, Vec<BackupContent>), Error> {

    let (manifest, mut files) = read_backup_index(&store, &info.backup_dir)?;

    let file_set = files.iter().fold(HashSet::new(), |mut acc, item| {
        acc.insert(item.filename.clone());
        acc
    });

    for file in &info.files {
        if file_set.contains(file) { continue; }
        files.push(BackupContent {
            filename: file.to_string(),
            size: None,
            crypt_mode: None,
        });
    }

    Ok((manifest, files))
}

#[api(
    input: {
        properties: {
            store: {
                schema: DATASTORE_SCHEMA,
            },
        },
    },
    returns: pbs_api_types::ADMIN_DATASTORE_LIST_GROUPS_RETURN_TYPE,
    access: {
        permission: &Permission::Privilege(
            &["datastore", "{store}"],
            PRIV_DATASTORE_AUDIT | PRIV_DATASTORE_BACKUP,
            true),
    },
)]
/// List backup groups.
pub fn list_groups(
    store: String,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<GroupListItem>, Error> {

    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let user_info = CachedUserInfo::new()?;
    let user_privs = user_info.lookup_privs(&auth_id, &["datastore", &store]);

    let datastore = DataStore::lookup_datastore(&store)?;
    let list_all = (user_privs & PRIV_DATASTORE_AUDIT) != 0;

    let backup_groups = BackupInfo::list_backup_groups(&datastore.base_path())?;

    let group_info = backup_groups
        .into_iter()
        .fold(Vec::new(), |mut group_info, group| {
            let owner = match datastore.get_owner(&group) {
                Ok(auth_id) => auth_id,
                Err(err) => {
                    eprintln!("Failed to get owner of group '{}/{}' - {}",
                             &store,
                             group,
                             err);
                    return group_info;
                },
            };
            if !list_all && check_backup_owner(&owner, &auth_id).is_err() {
                return group_info;
            }

            let snapshots = match group.list_backups(&datastore.base_path()) {
                Ok(snapshots) => snapshots,
                Err(_) => {
                    return group_info;
                },
            };

            let backup_count: u64 = snapshots.len() as u64;
            if backup_count == 0 {
                return group_info;
            }

            let last_backup = snapshots
                .iter()
                .fold(&snapshots[0], |last, curr| {
                    if curr.is_finished()
                        && curr.backup_dir.backup_time() > last.backup_dir.backup_time() {
                        curr
                    } else {
                        last
                    }
                })
                .to_owned();

            let note_path = get_group_note_path(&datastore, &group);
            let comment = file_read_firstline(&note_path).ok();

            group_info.push(GroupListItem {
                backup_type: group.backup_type().to_string(),
                backup_id: group.backup_id().to_string(),
                last_backup: last_backup.backup_dir.backup_time(),
                owner: Some(owner),
                backup_count,
                files: last_backup.files,
                comment,
            });

            group_info
        });

    Ok(group_info)
}

#[api(
    input: {
        properties: {
            store: {
                schema: DATASTORE_SCHEMA,
            },
            "backup-type": {
                schema: BACKUP_TYPE_SCHEMA,
            },
            "backup-id": {
                schema: BACKUP_ID_SCHEMA,
            },
        },
    },
    access: {
        permission: &Permission::Privilege(
            &["datastore", "{store}"],
            PRIV_DATASTORE_MODIFY| PRIV_DATASTORE_PRUNE,
            true),
    },
)]
/// Delete backup group including all snapshots.
pub fn delete_group(
    store: String,
    backup_type: String,
    backup_id: String,
    _info: &ApiMethod,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;

    let group = BackupGroup::new(backup_type, backup_id);
    let datastore = DataStore::lookup_datastore(&store)?;

    check_priv_or_backup_owner(&datastore, &group, &auth_id, PRIV_DATASTORE_MODIFY)?;

    datastore.remove_backup_group(&group)?;

    Ok(Value::Null)
}

#[api(
    input: {
        properties: {
            store: {
                schema: DATASTORE_SCHEMA,
            },
            "backup-type": {
                schema: BACKUP_TYPE_SCHEMA,
            },
            "backup-id": {
                schema: BACKUP_ID_SCHEMA,
            },
            "backup-time": {
                schema: BACKUP_TIME_SCHEMA,
            },
        },
    },
    returns: pbs_api_types::ADMIN_DATASTORE_LIST_SNAPSHOT_FILES_RETURN_TYPE,
    access: {
        permission: &Permission::Privilege(
            &["datastore", "{store}"],
            PRIV_DATASTORE_AUDIT | PRIV_DATASTORE_READ | PRIV_DATASTORE_BACKUP,
            true),
    },
)]
/// List snapshot files.
pub fn list_snapshot_files(
    store: String,
    backup_type: String,
    backup_id: String,
    backup_time: i64,
    _info: &ApiMethod,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<BackupContent>, Error> {

    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let datastore = DataStore::lookup_datastore(&store)?;

    let snapshot = BackupDir::new(backup_type, backup_id, backup_time)?;

    check_priv_or_backup_owner(&datastore, snapshot.group(), &auth_id, PRIV_DATASTORE_AUDIT | PRIV_DATASTORE_READ)?;

    let info = BackupInfo::new(&datastore.base_path(), snapshot)?;

    let (_manifest, files) = get_all_snapshot_files(&datastore, &info)?;

    Ok(files)
}

#[api(
    input: {
        properties: {
            store: {
                schema: DATASTORE_SCHEMA,
            },
            "backup-type": {
                schema: BACKUP_TYPE_SCHEMA,
            },
            "backup-id": {
                schema: BACKUP_ID_SCHEMA,
            },
            "backup-time": {
                schema: BACKUP_TIME_SCHEMA,
            },
        },
    },
    access: {
        permission: &Permission::Privilege(
            &["datastore", "{store}"],
            PRIV_DATASTORE_MODIFY| PRIV_DATASTORE_PRUNE,
            true),
    },
)]
/// Delete backup snapshot.
pub fn delete_snapshot(
    store: String,
    backup_type: String,
    backup_id: String,
    backup_time: i64,
    _info: &ApiMethod,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;

    let snapshot = BackupDir::new(backup_type, backup_id, backup_time)?;
    let datastore = DataStore::lookup_datastore(&store)?;

    check_priv_or_backup_owner(&datastore, snapshot.group(), &auth_id, PRIV_DATASTORE_MODIFY)?;

    datastore.remove_backup_dir(&snapshot, false)?;

    Ok(Value::Null)
}

#[api(
    input: {
        properties: {
            store: {
                schema: DATASTORE_SCHEMA,
            },
            "backup-type": {
                optional: true,
                schema: BACKUP_TYPE_SCHEMA,
            },
            "backup-id": {
                optional: true,
                schema: BACKUP_ID_SCHEMA,
            },
        },
    },
    returns: pbs_api_types::ADMIN_DATASTORE_LIST_SNAPSHOTS_RETURN_TYPE,
    access: {
        permission: &Permission::Privilege(
            &["datastore", "{store}"],
            PRIV_DATASTORE_AUDIT | PRIV_DATASTORE_BACKUP,
            true),
    },
)]
/// List backup snapshots.
pub fn list_snapshots (
    store: String,
    backup_type: Option<String>,
    backup_id: Option<String>,
    _param: Value,
    _info: &ApiMethod,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<SnapshotListItem>, Error> {

    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let user_info = CachedUserInfo::new()?;
    let user_privs = user_info.lookup_privs(&auth_id, &["datastore", &store]);

    let list_all = (user_privs & PRIV_DATASTORE_AUDIT) != 0;

    let datastore = DataStore::lookup_datastore(&store)?;

    let base_path = datastore.base_path();

    let groups = match (backup_type, backup_id) {
        (Some(backup_type), Some(backup_id)) => {
            let mut groups = Vec::with_capacity(1);
            groups.push(BackupGroup::new(backup_type, backup_id));
            groups
        },
        (Some(backup_type), None) => {
            BackupInfo::list_backup_groups(&base_path)?
                .into_iter()
                .filter(|group| group.backup_type() == backup_type)
                .collect()
        },
        (None, Some(backup_id)) => {
            BackupInfo::list_backup_groups(&base_path)?
                .into_iter()
                .filter(|group| group.backup_id() == backup_id)
                .collect()
        },
        _ => BackupInfo::list_backup_groups(&base_path)?,
    };

    let info_to_snapshot_list_item = |group: &BackupGroup, owner, info: BackupInfo| {
        let backup_type = group.backup_type().to_string();
        let backup_id = group.backup_id().to_string();
        let backup_time = info.backup_dir.backup_time();

        match get_all_snapshot_files(&datastore, &info) {
            Ok((manifest, files)) => {
                // extract the first line from notes
                let comment: Option<String> = manifest.unprotected["notes"]
                    .as_str()
                    .and_then(|notes| notes.lines().next())
                    .map(String::from);

                let fingerprint = match manifest.fingerprint() {
                    Ok(fp) => fp,
                    Err(err) => {
                        eprintln!("error parsing fingerprint: '{}'", err);
                        None
                    },
                };

                let verification = manifest.unprotected["verify_state"].clone();
                let verification: Option<SnapshotVerifyState> = match serde_json::from_value(verification) {
                    Ok(verify) => verify,
                    Err(err) => {
                        eprintln!("error parsing verification state : '{}'", err);
                        None
                    }
                };

                let size = Some(files.iter().map(|x| x.size.unwrap_or(0)).sum());

                SnapshotListItem {
                    backup_type,
                    backup_id,
                    backup_time,
                    comment,
                    verification,
                    fingerprint,
                    files,
                    size,
                    owner,
                }
            },
            Err(err) => {
                eprintln!("error during snapshot file listing: '{}'", err);
                let files = info
                        .files
                        .into_iter()
                        .map(|filename| BackupContent {
                            filename,
                            size: None,
                            crypt_mode: None,
                        })
                        .collect();

                SnapshotListItem {
                    backup_type,
                    backup_id,
                    backup_time,
                    comment: None,
                    verification: None,
                    fingerprint: None,
                    files,
                    size: None,
                    owner,
                }
            },
        }
    };

    groups
        .iter()
        .try_fold(Vec::new(), |mut snapshots, group| {
            let owner = match datastore.get_owner(group) {
                Ok(auth_id) => auth_id,
                Err(err) => {
                    eprintln!("Failed to get owner of group '{}/{}' - {}",
                              &store,
                              group,
                              err);
                    return Ok(snapshots);
                },
            };

            if !list_all && check_backup_owner(&owner, &auth_id).is_err() {
                return Ok(snapshots);
            }

            let group_backups = group.list_backups(&datastore.base_path())?;

            snapshots.extend(
                group_backups
                    .into_iter()
                    .map(|info| info_to_snapshot_list_item(&group, Some(owner.clone()), info))
            );

            Ok(snapshots)
        })
}

fn get_snapshots_count(store: &DataStore, filter_owner: Option<&Authid>) -> Result<Counts, Error> {
    let base_path = store.base_path();
    let groups = BackupInfo::list_backup_groups(&base_path)?;

    groups.iter()
        .filter(|group| {
            let owner = match store.get_owner(&group) {
                Ok(owner) => owner,
                Err(err) => {
                    eprintln!("Failed to get owner of group '{}/{}' - {}",
                              store.name(),
                              group,
                              err);
                    return false;
                },
            };

            match filter_owner {
                Some(filter) => check_backup_owner(&owner, filter).is_ok(),
                None => true,
            }
        })
        .try_fold(Counts::default(), |mut counts, group| {
            let snapshot_count = group.list_backups(&base_path)?.len() as u64;

            let type_count = match group.backup_type() {
                "ct" => counts.ct.get_or_insert(Default::default()),
                "vm" => counts.vm.get_or_insert(Default::default()),
                "host" => counts.host.get_or_insert(Default::default()),
                _ => counts.other.get_or_insert(Default::default()),
            };

            type_count.groups += 1;
            type_count.snapshots += snapshot_count;

            Ok(counts)
        })
}

#[api(
    input: {
        properties: {
            store: {
                schema: DATASTORE_SCHEMA,
            },
            verbose: {
                type: bool,
                default: false,
                optional: true,
                description: "Include additional information like snapshot counts and GC status.",
            },
        },

    },
    returns: {
        type: DataStoreStatus,
    },
    access: {
        permission: &Permission::Privilege(&["datastore", "{store}"], PRIV_DATASTORE_AUDIT | PRIV_DATASTORE_BACKUP, true),
    },
)]
/// Get datastore status.
pub fn status(
    store: String,
    verbose: bool,
    _info: &ApiMethod,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<DataStoreStatus, Error> {
    let datastore = DataStore::lookup_datastore(&store)?;
    let storage = crate::tools::disks::disk_usage(&datastore.base_path())?;
    let (counts, gc_status) = if verbose {
        let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
        let user_info = CachedUserInfo::new()?;

        let store_privs = user_info.lookup_privs(&auth_id, &["datastore", &store]);
        let filter_owner = if store_privs & PRIV_DATASTORE_AUDIT != 0 {
            None
        } else {
            Some(&auth_id)
        };

        let counts = Some(get_snapshots_count(&datastore, filter_owner)?);
        let gc_status = Some(datastore.last_gc_status());

        (counts, gc_status)
    } else {
        (None, None)
    };

    Ok(DataStoreStatus {
        total: storage.total,
        used: storage.used,
        avail: storage.avail,
        gc_status,
        counts,
    })
}

#[api(
    input: {
        properties: {
            store: {
                schema: DATASTORE_SCHEMA,
            },
            "backup-type": {
                schema: BACKUP_TYPE_SCHEMA,
                optional: true,
            },
            "backup-id": {
                schema: BACKUP_ID_SCHEMA,
                optional: true,
            },
            "ignore-verified": {
                schema: IGNORE_VERIFIED_BACKUPS_SCHEMA,
                optional: true,
            },
            "outdated-after": {
                schema: VERIFICATION_OUTDATED_AFTER_SCHEMA,
                optional: true,
            },
            "backup-time": {
                schema: BACKUP_TIME_SCHEMA,
                optional: true,
            },
        },
    },
    returns: {
        schema: UPID_SCHEMA,
    },
    access: {
        permission: &Permission::Privilege(&["datastore", "{store}"], PRIV_DATASTORE_VERIFY | PRIV_DATASTORE_BACKUP, true),
    },
)]
/// Verify backups.
///
/// This function can verify a single backup snapshot, all backup from a backup group,
/// or all backups in the datastore.
pub fn verify(
    store: String,
    backup_type: Option<String>,
    backup_id: Option<String>,
    backup_time: Option<i64>,
    ignore_verified: Option<bool>,
    outdated_after: Option<i64>,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {
    let datastore = DataStore::lookup_datastore(&store)?;
    let ignore_verified = ignore_verified.unwrap_or(true);

    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let worker_id;

    let mut backup_dir = None;
    let mut backup_group = None;
    let mut worker_type = "verify";

    match (backup_type, backup_id, backup_time) {
        (Some(backup_type), Some(backup_id), Some(backup_time)) => {
            worker_id = format!("{}:{}/{}/{:08X}", store, backup_type, backup_id, backup_time);
            let dir = BackupDir::new(backup_type, backup_id, backup_time)?;

            check_priv_or_backup_owner(&datastore, dir.group(), &auth_id, PRIV_DATASTORE_VERIFY)?;

            backup_dir = Some(dir);
            worker_type = "verify_snapshot";
        }
        (Some(backup_type), Some(backup_id), None) => {
            worker_id = format!("{}:{}/{}", store, backup_type, backup_id);
            let group = BackupGroup::new(backup_type, backup_id);

            check_priv_or_backup_owner(&datastore, &group, &auth_id, PRIV_DATASTORE_VERIFY)?;

            backup_group = Some(group);
            worker_type = "verify_group";
        }
        (None, None, None) => {
            worker_id = store.clone();
        }
        _ => bail!("parameters do not specify a backup group or snapshot"),
    }

    let to_stdout = rpcenv.env_type() == RpcEnvironmentType::CLI;

    let upid_str = WorkerTask::new_thread(
        worker_type,
        Some(worker_id),
        auth_id.clone(),
        to_stdout,
        move |worker| {
            let verify_worker = crate::backup::VerifyWorker::new(worker.clone(), datastore);
            let failed_dirs = if let Some(backup_dir) = backup_dir {
                let mut res = Vec::new();
                if !verify_backup_dir(
                    &verify_worker,
                    &backup_dir,
                    worker.upid().clone(),
                    Some(&move |manifest| {
                        verify_filter(ignore_verified, outdated_after, manifest)
                    }),
                )? {
                    res.push(backup_dir.to_string());
                }
                res
            } else if let Some(backup_group) = backup_group {
                let failed_dirs = verify_backup_group(
                    &verify_worker,
                    &backup_group,
                    &mut StoreProgress::new(1),
                    worker.upid(),
                    Some(&move |manifest| {
                        verify_filter(ignore_verified, outdated_after, manifest)
                    }),
                )?;
                failed_dirs
            } else {
                let privs = CachedUserInfo::new()?
                    .lookup_privs(&auth_id, &["datastore", &store]);

                let owner = if privs & PRIV_DATASTORE_VERIFY == 0 {
                    Some(auth_id)
                } else {
                    None
                };

                verify_all_backups(
                    &verify_worker,
                    worker.upid(),
                    owner,
                    Some(&move |manifest| {
                        verify_filter(ignore_verified, outdated_after, manifest)
                    }),
                )?
            };
            if !failed_dirs.is_empty() {
                worker.log("Failed to verify the following snapshots/groups:");
                for dir in failed_dirs {
                    worker.log(format!("\t{}", dir));
                }
                bail!("verification failed - please check the log for details");
            }
            Ok(())
        },
    )?;

    Ok(json!(upid_str))
}

#[api(
    input: {
        properties: {
            "backup-id": {
                schema: BACKUP_ID_SCHEMA,
            },
            "backup-type": {
                schema: BACKUP_TYPE_SCHEMA,
            },
            "dry-run": {
                optional: true,
                type: bool,
                default: false,
                description: "Just show what prune would do, but do not delete anything.",
            },
            "prune-options": {
                type: PruneOptions,
                flatten: true,
            },
            store: {
                schema: DATASTORE_SCHEMA,
            },
        },
    },
    returns: pbs_api_types::ADMIN_DATASTORE_PRUNE_RETURN_TYPE,
    access: {
        permission: &Permission::Privilege(&["datastore", "{store}"], PRIV_DATASTORE_MODIFY | PRIV_DATASTORE_PRUNE, true),
    },
)]
/// Prune a group on the datastore
pub fn prune(
    backup_id: String,
    backup_type: String,
    dry_run: bool,
    prune_options: PruneOptions,
    store: String,
    _param: Value,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;

    let group = BackupGroup::new(&backup_type, &backup_id);

    let datastore = DataStore::lookup_datastore(&store)?;

    check_priv_or_backup_owner(&datastore, &group, &auth_id, PRIV_DATASTORE_MODIFY)?;

    let worker_id = format!("{}:{}/{}", store, &backup_type, &backup_id);

    let mut prune_result = Vec::new();

    let list = group.list_backups(&datastore.base_path())?;

    let mut prune_info = compute_prune_info(list, &prune_options)?;

    prune_info.reverse(); // delete older snapshots first

    let keep_all = !pbs_datastore::prune::keeps_something(&prune_options);

    if dry_run {
        for (info, mut keep) in prune_info {
            if keep_all { keep = true; }

            let backup_time = info.backup_dir.backup_time();
            let group = info.backup_dir.group();

            prune_result.push(json!({
                "backup-type": group.backup_type(),
                "backup-id": group.backup_id(),
                "backup-time": backup_time,
                "keep": keep,
            }));
        }
        return Ok(json!(prune_result));
    }


    // We use a WorkerTask just to have a task log, but run synchrounously
    let worker = WorkerTask::new("prune", Some(worker_id), auth_id, true)?;

    if keep_all {
        worker.log("No prune selection - keeping all files.");
    } else {
        worker.log(format!("retention options: {}", pbs_datastore::prune::cli_options_string(&prune_options)));
        worker.log(format!("Starting prune on store \"{}\" group \"{}/{}\"",
                            store, backup_type, backup_id));
    }

    for (info, mut keep) in prune_info {
        if keep_all { keep = true; }

        let backup_time = info.backup_dir.backup_time();
        let timestamp = info.backup_dir.backup_time_string();
        let group = info.backup_dir.group();


        let msg = format!(
            "{}/{}/{} {}",
            group.backup_type(),
            group.backup_id(),
            timestamp,
            if keep { "keep" } else { "remove" },
        );

        worker.log(msg);

        prune_result.push(json!({
            "backup-type": group.backup_type(),
            "backup-id": group.backup_id(),
            "backup-time": backup_time,
            "keep": keep,
        }));

        if !(dry_run || keep) {
            if let Err(err) = datastore.remove_backup_dir(&info.backup_dir, false) {
                worker.warn(
                    format!(
                        "failed to remove dir {:?}: {}",
                        info.backup_dir.relative_path(), err
                    )
                );
            }
        }
    }

    worker.log_result(&Ok(()));

    Ok(json!(prune_result))
}

#[api(
    input: {
        properties: {
            "dry-run": {
                optional: true,
                type: bool,
                default: false,
                description: "Just show what prune would do, but do not delete anything.",
            },
            "prune-options": {
                type: PruneOptions,
                flatten: true,
            },
            store: {
                schema: DATASTORE_SCHEMA,
            },
        },
    },
    returns: {
        schema: UPID_SCHEMA,
    },
    access: {
        permission: &Permission::Privilege(&["datastore", "{store}"], PRIV_DATASTORE_MODIFY | PRIV_DATASTORE_PRUNE, true),
    },
)]
/// Prune the datastore
pub fn prune_datastore(
    dry_run: bool,
    prune_options: PruneOptions,
    store: String,
    _param: Value,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<String, Error> {

    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;

    let datastore = DataStore::lookup_datastore(&store)?;

    let to_stdout = rpcenv.env_type() == RpcEnvironmentType::CLI;

    let upid_str = WorkerTask::new_thread(
        "prune",
        Some(store.clone()),
        auth_id.clone(),
        to_stdout,
        move |worker| crate::server::prune_datastore(
            worker.clone(),
            auth_id,
            prune_options,
            &store,
            datastore,
            dry_run
        ),
    )?;

    Ok(upid_str)
}

#[api(
    input: {
        properties: {
            store: {
                schema: DATASTORE_SCHEMA,
            },
        },
    },
    returns: {
        schema: UPID_SCHEMA,
    },
    access: {
        permission: &Permission::Privilege(&["datastore", "{store}"], PRIV_DATASTORE_MODIFY, false),
    },
)]
/// Start garbage collection.
pub fn start_garbage_collection(
    store: String,
    _info: &ApiMethod,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let datastore = DataStore::lookup_datastore(&store)?;
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;

    let job =  Job::new("garbage_collection", &store)
        .map_err(|_| format_err!("garbage collection already running"))?;

    let to_stdout = rpcenv.env_type() == RpcEnvironmentType::CLI;

    let upid_str = crate::server::do_garbage_collection_job(job, datastore, &auth_id, None, to_stdout)
        .map_err(|err| format_err!("unable to start garbage collection job on datastore {} - {}", store, err))?;

    Ok(json!(upid_str))
}

#[api(
    input: {
        properties: {
            store: {
                schema: DATASTORE_SCHEMA,
            },
        },
    },
    returns: {
        type: GarbageCollectionStatus,
    },
    access: {
        permission: &Permission::Privilege(&["datastore", "{store}"], PRIV_DATASTORE_AUDIT, false),
    },
)]
/// Garbage collection status.
pub fn garbage_collection_status(
    store: String,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<GarbageCollectionStatus, Error> {

    let datastore = DataStore::lookup_datastore(&store)?;

    let status = datastore.last_gc_status();

    Ok(status)
}

#[api(
    returns: {
        description: "List the accessible datastores.",
        type: Array,
        items: { type: DataStoreListItem },
    },
    access: {
        permission: &Permission::Anybody,
    },
)]
/// Datastore list
pub fn get_datastore_list(
    _param: Value,
    _info: &ApiMethod,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<DataStoreListItem>, Error> {

    let (config, _digest) = pbs_config::datastore::config()?;

    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let user_info = CachedUserInfo::new()?;

    let mut list = Vec::new();

    for (store, (_, data)) in &config.sections {
        let user_privs = user_info.lookup_privs(&auth_id, &["datastore", &store]);
        let allowed = (user_privs & (PRIV_DATASTORE_AUDIT| PRIV_DATASTORE_BACKUP)) != 0;
        if allowed {
            list.push(
                DataStoreListItem {
                    store: store.clone(),
                    comment: data["comment"].as_str().map(String::from),
                }
            );
        }
    }

    Ok(list)
}

#[sortable]
pub const API_METHOD_DOWNLOAD_FILE: ApiMethod = ApiMethod::new(
    &ApiHandler::AsyncHttp(&download_file),
    &ObjectSchema::new(
        "Download single raw file from backup snapshot.",
        &sorted!([
            ("store", false, &DATASTORE_SCHEMA),
            ("backup-type", false, &BACKUP_TYPE_SCHEMA),
            ("backup-id", false,  &BACKUP_ID_SCHEMA),
            ("backup-time", false, &BACKUP_TIME_SCHEMA),
            ("file-name", false, &BACKUP_ARCHIVE_NAME_SCHEMA),
        ]),
    )
).access(None, &Permission::Privilege(
    &["datastore", "{store}"],
    PRIV_DATASTORE_READ | PRIV_DATASTORE_BACKUP,
    true)
);

pub fn download_file(
    _parts: Parts,
    _req_body: Body,
    param: Value,
    _info: &ApiMethod,
    rpcenv: Box<dyn RpcEnvironment>,
) -> ApiResponseFuture {

    async move {
        let store = required_string_param(&param, "store")?;
        let datastore = DataStore::lookup_datastore(store)?;

        let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;

        let file_name = required_string_param(&param, "file-name")?.to_owned();

        let backup_type = required_string_param(&param, "backup-type")?;
        let backup_id = required_string_param(&param, "backup-id")?;
        let backup_time = required_integer_param(&param, "backup-time")?;

        let backup_dir = BackupDir::new(backup_type, backup_id, backup_time)?;

        check_priv_or_backup_owner(&datastore, backup_dir.group(), &auth_id, PRIV_DATASTORE_READ)?;

        println!("Download {} from {} ({}/{})", file_name, store, backup_dir, file_name);

        let mut path = datastore.base_path();
        path.push(backup_dir.relative_path());
        path.push(&file_name);

        let file = tokio::fs::File::open(&path)
            .await
            .map_err(|err| http_err!(BAD_REQUEST, "File open failed: {}", err))?;

        let payload = tokio_util::codec::FramedRead::new(file, tokio_util::codec::BytesCodec::new())
            .map_ok(|bytes| bytes.freeze())
            .map_err(move |err| {
                eprintln!("error during streaming of '{:?}' - {}", &path, err);
                err
            });
        let body = Body::wrap_stream(payload);

        // fixme: set other headers ?
        Ok(Response::builder()
           .status(StatusCode::OK)
           .header(header::CONTENT_TYPE, "application/octet-stream")
           .body(body)
           .unwrap())
    }.boxed()
}

#[sortable]
pub const API_METHOD_DOWNLOAD_FILE_DECODED: ApiMethod = ApiMethod::new(
    &ApiHandler::AsyncHttp(&download_file_decoded),
    &ObjectSchema::new(
        "Download single decoded file from backup snapshot. Only works if it's not encrypted.",
        &sorted!([
            ("store", false, &DATASTORE_SCHEMA),
            ("backup-type", false, &BACKUP_TYPE_SCHEMA),
            ("backup-id", false,  &BACKUP_ID_SCHEMA),
            ("backup-time", false, &BACKUP_TIME_SCHEMA),
            ("file-name", false, &BACKUP_ARCHIVE_NAME_SCHEMA),
        ]),
    )
).access(None, &Permission::Privilege(
    &["datastore", "{store}"],
    PRIV_DATASTORE_READ | PRIV_DATASTORE_BACKUP,
    true)
);

pub fn download_file_decoded(
    _parts: Parts,
    _req_body: Body,
    param: Value,
    _info: &ApiMethod,
    rpcenv: Box<dyn RpcEnvironment>,
) -> ApiResponseFuture {

    async move {
        let store = required_string_param(&param, "store")?;
        let datastore = DataStore::lookup_datastore(store)?;

        let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;

        let file_name = required_string_param(&param, "file-name")?.to_owned();

        let backup_type = required_string_param(&param, "backup-type")?;
        let backup_id = required_string_param(&param, "backup-id")?;
        let backup_time = required_integer_param(&param, "backup-time")?;

        let backup_dir = BackupDir::new(backup_type, backup_id, backup_time)?;

        check_priv_or_backup_owner(&datastore, backup_dir.group(), &auth_id, PRIV_DATASTORE_READ)?;

        let (manifest, files) = read_backup_index(&datastore, &backup_dir)?;
        for file in files {
            if file.filename == file_name && file.crypt_mode == Some(CryptMode::Encrypt) {
                bail!("cannot decode '{}' - is encrypted", file_name);
            }
        }

        println!("Download {} from {} ({}/{})", file_name, store, backup_dir, file_name);

        let mut path = datastore.base_path();
        path.push(backup_dir.relative_path());
        path.push(&file_name);

        let extension = file_name.rsplitn(2, '.').next().unwrap();

        let body = match extension {
            "didx" => {
                let index = DynamicIndexReader::open(&path)
                    .map_err(|err| format_err!("unable to read dynamic index '{:?}' - {}", &path, err))?;
                let (csum, size) = index.compute_csum();
                manifest.verify_file(&file_name, &csum, size)?;

                let chunk_reader = LocalChunkReader::new(datastore, None, CryptMode::None);
                let reader = CachedChunkReader::new(chunk_reader, index, 1).seekable();
                Body::wrap_stream(AsyncReaderStream::new(reader)
                    .map_err(move |err| {
                        eprintln!("error during streaming of '{:?}' - {}", path, err);
                        err
                    }))
            },
            "fidx" => {
                let index = FixedIndexReader::open(&path)
                    .map_err(|err| format_err!("unable to read fixed index '{:?}' - {}", &path, err))?;

                let (csum, size) = index.compute_csum();
                manifest.verify_file(&file_name, &csum, size)?;

                let chunk_reader = LocalChunkReader::new(datastore, None, CryptMode::None);
                let reader = CachedChunkReader::new(chunk_reader, index, 1).seekable();
                Body::wrap_stream(AsyncReaderStream::with_buffer_size(reader, 4*1024*1024)
                    .map_err(move |err| {
                        eprintln!("error during streaming of '{:?}' - {}", path, err);
                        err
                    }))
            },
            "blob" => {
                let file = std::fs::File::open(&path)
                    .map_err(|err| http_err!(BAD_REQUEST, "File open failed: {}", err))?;

                // FIXME: load full blob to verify index checksum?

                Body::wrap_stream(
                    WrappedReaderStream::new(DataBlobReader::new(file, None)?)
                        .map_err(move |err| {
                            eprintln!("error during streaming of '{:?}' - {}", path, err);
                            err
                        })
                )
            },
            extension => {
                bail!("cannot download '{}' files", extension);
            },
        };

        // fixme: set other headers ?
        Ok(Response::builder()
           .status(StatusCode::OK)
           .header(header::CONTENT_TYPE, "application/octet-stream")
           .body(body)
           .unwrap())
    }.boxed()
}

#[sortable]
pub const API_METHOD_UPLOAD_BACKUP_LOG: ApiMethod = ApiMethod::new(
    &ApiHandler::AsyncHttp(&upload_backup_log),
    &ObjectSchema::new(
        "Upload the client backup log file into a backup snapshot ('client.log.blob').",
        &sorted!([
            ("store", false, &DATASTORE_SCHEMA),
            ("backup-type", false, &BACKUP_TYPE_SCHEMA),
            ("backup-id", false, &BACKUP_ID_SCHEMA),
            ("backup-time", false, &BACKUP_TIME_SCHEMA),
        ]),
    )
).access(
    Some("Only the backup creator/owner is allowed to do this."),
    &Permission::Privilege(&["datastore", "{store}"], PRIV_DATASTORE_BACKUP, false)
);

pub fn upload_backup_log(
    _parts: Parts,
    req_body: Body,
    param: Value,
    _info: &ApiMethod,
    rpcenv: Box<dyn RpcEnvironment>,
) -> ApiResponseFuture {

    async move {
        let store = required_string_param(&param, "store")?;
        let datastore = DataStore::lookup_datastore(store)?;

        let file_name =  CLIENT_LOG_BLOB_NAME;

        let backup_type = required_string_param(&param, "backup-type")?;
        let backup_id = required_string_param(&param, "backup-id")?;
        let backup_time = required_integer_param(&param, "backup-time")?;

        let backup_dir = BackupDir::new(backup_type, backup_id, backup_time)?;

        let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
        let owner = datastore.get_owner(backup_dir.group())?;
        check_backup_owner(&owner, &auth_id)?;

        let mut path = datastore.base_path();
        path.push(backup_dir.relative_path());
        path.push(&file_name);

        if path.exists() {
            bail!("backup already contains a log.");
        }

        println!("Upload backup log to {}/{}/{}/{}/{}", store,
                 backup_type, backup_id, backup_dir.backup_time_string(), file_name);

        let data = req_body
            .map_err(Error::from)
            .try_fold(Vec::new(), |mut acc, chunk| {
                acc.extend_from_slice(&*chunk);
                future::ok::<_, Error>(acc)
            })
            .await?;

        // always verify blob/CRC at server side
        let blob = DataBlob::load_from_reader(&mut &data[..])?;

        replace_file(&path, blob.raw_data(), CreateOptions::new())?;

        // fixme: use correct formatter
        Ok(formatter::json_response(Ok(Value::Null)))
    }.boxed()
}

#[api(
    input: {
        properties: {
            store: {
                schema: DATASTORE_SCHEMA,
            },
            "backup-type": {
                schema: BACKUP_TYPE_SCHEMA,
            },
            "backup-id": {
                schema: BACKUP_ID_SCHEMA,
            },
            "backup-time": {
                schema: BACKUP_TIME_SCHEMA,
            },
            "filepath": {
                description: "Base64 encoded path.",
                type: String,
            }
        },
    },
    access: {
        permission: &Permission::Privilege(&["datastore", "{store}"], PRIV_DATASTORE_READ | PRIV_DATASTORE_BACKUP, true),
    },
)]
/// Get the entries of the given path of the catalog
pub fn catalog(
    store: String,
    backup_type: String,
    backup_id: String,
    backup_time: i64,
    filepath: String,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<ArchiveEntry>, Error> {
    let datastore = DataStore::lookup_datastore(&store)?;

    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;

    let backup_dir = BackupDir::new(backup_type, backup_id, backup_time)?;

    check_priv_or_backup_owner(&datastore, backup_dir.group(), &auth_id, PRIV_DATASTORE_READ)?;

    let file_name = CATALOG_NAME;

    let (manifest, files) = read_backup_index(&datastore, &backup_dir)?;
    for file in files {
        if file.filename == file_name && file.crypt_mode == Some(CryptMode::Encrypt) {
            bail!("cannot decode '{}' - is encrypted", file_name);
        }
    }

    let mut path = datastore.base_path();
    path.push(backup_dir.relative_path());
    path.push(file_name);

    let index = DynamicIndexReader::open(&path)
        .map_err(|err| format_err!("unable to read dynamic index '{:?}' - {}", &path, err))?;

    let (csum, size) = index.compute_csum();
    manifest.verify_file(&file_name, &csum, size)?;

    let chunk_reader = LocalChunkReader::new(datastore, None, CryptMode::None);
    let reader = BufferedDynamicReader::new(index, chunk_reader);

    let mut catalog_reader = CatalogReader::new(reader);

    let path = if filepath != "root" && filepath != "/" {
        base64::decode(filepath)?
    } else {
        vec![b'/']
    };

    catalog_reader.list_dir_contents(&path)
}

#[sortable]
pub const API_METHOD_PXAR_FILE_DOWNLOAD: ApiMethod = ApiMethod::new(
    &ApiHandler::AsyncHttp(&pxar_file_download),
    &ObjectSchema::new(
        "Download single file from pxar file of a backup snapshot. Only works if it's not encrypted.",
        &sorted!([
            ("store", false, &DATASTORE_SCHEMA),
            ("backup-type", false, &BACKUP_TYPE_SCHEMA),
            ("backup-id", false,  &BACKUP_ID_SCHEMA),
            ("backup-time", false, &BACKUP_TIME_SCHEMA),
            ("filepath", false, &StringSchema::new("Base64 encoded path").schema()),
        ]),
    )
).access(None, &Permission::Privilege(
    &["datastore", "{store}"],
    PRIV_DATASTORE_READ | PRIV_DATASTORE_BACKUP,
    true)
);

pub fn pxar_file_download(
    _parts: Parts,
    _req_body: Body,
    param: Value,
    _info: &ApiMethod,
    rpcenv: Box<dyn RpcEnvironment>,
) -> ApiResponseFuture {

    async move {
        let store = required_string_param(&param, "store")?;
        let datastore = DataStore::lookup_datastore(&store)?;

        let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;

        let filepath = required_string_param(&param, "filepath")?.to_owned();

        let backup_type = required_string_param(&param, "backup-type")?;
        let backup_id = required_string_param(&param, "backup-id")?;
        let backup_time = required_integer_param(&param, "backup-time")?;

        let backup_dir = BackupDir::new(backup_type, backup_id, backup_time)?;

        check_priv_or_backup_owner(&datastore, backup_dir.group(), &auth_id, PRIV_DATASTORE_READ)?;

        let mut components = base64::decode(&filepath)?;
        if !components.is_empty() && components[0] == b'/' {
            components.remove(0);
        }

        let mut split = components.splitn(2, |c| *c == b'/');
        let pxar_name = std::str::from_utf8(split.next().unwrap())?;
        let file_path = split.next().unwrap_or(b"/");
        let (manifest, files) = read_backup_index(&datastore, &backup_dir)?;
        for file in files {
            if file.filename == pxar_name && file.crypt_mode == Some(CryptMode::Encrypt) {
                bail!("cannot decode '{}' - is encrypted", pxar_name);
            }
        }

        let mut path = datastore.base_path();
        path.push(backup_dir.relative_path());
        path.push(pxar_name);

        let index = DynamicIndexReader::open(&path)
            .map_err(|err| format_err!("unable to read dynamic index '{:?}' - {}", &path, err))?;

        let (csum, size) = index.compute_csum();
        manifest.verify_file(&pxar_name, &csum, size)?;

        let chunk_reader = LocalChunkReader::new(datastore, None, CryptMode::None);
        let reader = BufferedDynamicReader::new(index, chunk_reader);
        let archive_size = reader.archive_size();
        let reader = LocalDynamicReadAt::new(reader);

        let decoder = Accessor::new(reader, archive_size).await?;
        let root = decoder.open_root().await?;
        let path = OsStr::from_bytes(file_path).to_os_string();
        let file = root
            .lookup(&path).await?
            .ok_or_else(|| format_err!("error opening '{:?}'", path))?;

        let body = match file.kind() {
            EntryKind::File { .. } => Body::wrap_stream(
                AsyncReaderStream::new(file.contents().await?).map_err(move |err| {
                    eprintln!("error during streaming of file '{:?}' - {}", filepath, err);
                    err
                }),
            ),
            EntryKind::Hardlink(_) => Body::wrap_stream(
                AsyncReaderStream::new(decoder.follow_hardlink(&file).await?.contents().await?)
                    .map_err(move |err| {
                        eprintln!(
                            "error during streaming of hardlink '{:?}' - {}",
                            path, err
                        );
                        err
                    }),
            ),
            EntryKind::Directory => {
                let (sender, receiver) = tokio::sync::mpsc::channel(100);
                let channelwriter = AsyncChannelWriter::new(sender, 1024 * 1024);
                proxmox_rest_server::spawn_internal_task(
                    create_zip(channelwriter, decoder, path.clone(), false)
                );
                Body::wrap_stream(ReceiverStream::new(receiver).map_err(move |err| {
                    eprintln!("error during streaming of zip '{:?}' - {}", path, err);
                    err
                }))
            }
            other => bail!("cannot download file of type {:?}", other),
        };

        // fixme: set other headers ?
        Ok(Response::builder()
           .status(StatusCode::OK)
           .header(header::CONTENT_TYPE, "application/octet-stream")
           .body(body)
           .unwrap())
    }.boxed()
}

#[api(
    input: {
        properties: {
            store: {
                schema: DATASTORE_SCHEMA,
            },
            timeframe: {
                type: RRDTimeFrameResolution,
            },
            cf: {
                type: RRDMode,
            },
        },
    },
    access: {
        permission: &Permission::Privilege(&["datastore", "{store}"], PRIV_DATASTORE_AUDIT | PRIV_DATASTORE_BACKUP, true),
    },
)]
/// Read datastore stats
pub fn get_rrd_stats(
    store: String,
    timeframe: RRDTimeFrameResolution,
    cf: RRDMode,
    _param: Value,
) -> Result<Value, Error> {

    create_value_from_rrd(
        &format!("datastore/{}", store),
        &[
            "total", "used",
            "read_ios", "read_bytes",
            "write_ios", "write_bytes",
            "io_ticks",
        ],
        timeframe,
        cf,
    )
}

#[api(
    input: {
        properties: {
            store: {
                schema: DATASTORE_SCHEMA,
            },
            "backup-type": {
                schema: BACKUP_TYPE_SCHEMA,
            },
            "backup-id": {
                schema: BACKUP_ID_SCHEMA,
            },
        },
    },
    access: {
        permission: &Permission::Privilege(&["datastore", "{store}"], PRIV_DATASTORE_AUDIT | PRIV_DATASTORE_BACKUP, true),
    },
)]
/// Get "notes" for a backup group
pub fn get_group_notes(
    store: String,
    backup_type: String,
    backup_id: String,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<String, Error> {
    let datastore = DataStore::lookup_datastore(&store)?;

    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let backup_group = BackupGroup::new(backup_type, backup_id);

    check_priv_or_backup_owner(&datastore, &backup_group, &auth_id, PRIV_DATASTORE_AUDIT)?;

    let note_path = get_group_note_path(&datastore, &backup_group);
    Ok(file_read_optional_string(note_path)?.unwrap_or_else(|| "".to_owned()))
}

#[api(
    input: {
        properties: {
            store: {
                schema: DATASTORE_SCHEMA,
            },
            "backup-type": {
                schema: BACKUP_TYPE_SCHEMA,
            },
            "backup-id": {
                schema: BACKUP_ID_SCHEMA,
            },
            notes: {
                description: "A multiline text.",
            },
        },
    },
    access: {
        permission: &Permission::Privilege(&["datastore", "{store}"],
                                           PRIV_DATASTORE_MODIFY | PRIV_DATASTORE_BACKUP,
                                           true),
    },
)]
/// Set "notes" for a backup group
pub fn set_group_notes(
    store: String,
    backup_type: String,
    backup_id: String,
    notes: String,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {
    let datastore = DataStore::lookup_datastore(&store)?;

    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let backup_group = BackupGroup::new(backup_type, backup_id);

    check_priv_or_backup_owner(&datastore, &backup_group, &auth_id, PRIV_DATASTORE_MODIFY)?;

    let note_path = get_group_note_path(&datastore, &backup_group);
    replace_file(note_path, notes.as_bytes(), CreateOptions::new())?;

    Ok(())
}

#[api(
    input: {
        properties: {
            store: {
                schema: DATASTORE_SCHEMA,
            },
            "backup-type": {
                schema: BACKUP_TYPE_SCHEMA,
            },
            "backup-id": {
                schema: BACKUP_ID_SCHEMA,
            },
            "backup-time": {
                schema: BACKUP_TIME_SCHEMA,
            },
        },
    },
    access: {
        permission: &Permission::Privilege(&["datastore", "{store}"], PRIV_DATASTORE_AUDIT | PRIV_DATASTORE_BACKUP, true),
    },
)]
/// Get "notes" for a specific backup
pub fn get_notes(
    store: String,
    backup_type: String,
    backup_id: String,
    backup_time: i64,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<String, Error> {
    let datastore = DataStore::lookup_datastore(&store)?;

    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let backup_dir = BackupDir::new(backup_type, backup_id, backup_time)?;

    check_priv_or_backup_owner(&datastore, backup_dir.group(), &auth_id, PRIV_DATASTORE_AUDIT)?;

    let (manifest, _) = datastore.load_manifest(&backup_dir)?;

    let notes = manifest.unprotected["notes"]
        .as_str()
        .unwrap_or("");

    Ok(String::from(notes))
}

#[api(
    input: {
        properties: {
            store: {
                schema: DATASTORE_SCHEMA,
            },
            "backup-type": {
                schema: BACKUP_TYPE_SCHEMA,
            },
            "backup-id": {
                schema: BACKUP_ID_SCHEMA,
            },
            "backup-time": {
                schema: BACKUP_TIME_SCHEMA,
            },
            notes: {
                description: "A multiline text.",
            },
        },
    },
    access: {
        permission: &Permission::Privilege(&["datastore", "{store}"],
                                           PRIV_DATASTORE_MODIFY | PRIV_DATASTORE_BACKUP,
                                           true),
    },
)]
/// Set "notes" for a specific backup
pub fn set_notes(
    store: String,
    backup_type: String,
    backup_id: String,
    backup_time: i64,
    notes: String,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {
    let datastore = DataStore::lookup_datastore(&store)?;

    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let backup_dir = BackupDir::new(backup_type, backup_id, backup_time)?;

    check_priv_or_backup_owner(&datastore, backup_dir.group(), &auth_id, PRIV_DATASTORE_MODIFY)?;

    datastore.update_manifest(&backup_dir,|manifest| {
        manifest.unprotected["notes"] = notes.into();
    }).map_err(|err| format_err!("unable to update manifest blob - {}", err))?;

    Ok(())
}

#[api(
    input: {
        properties: {
            store: {
                schema: DATASTORE_SCHEMA,
            },
            "backup-type": {
                schema: BACKUP_TYPE_SCHEMA,
            },
            "backup-id": {
                schema: BACKUP_ID_SCHEMA,
            },
            "new-owner": {
                type: Authid,
            },
        },
    },
    access: {
        permission: &Permission::Anybody,
        description: "Datastore.Modify on whole datastore, or changing ownership between user and a user's token for owned backups with Datastore.Backup"
    },
)]
/// Change owner of a backup group
pub fn set_backup_owner(
    store: String,
    backup_type: String,
    backup_id: String,
    new_owner: Authid,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {

    let datastore = DataStore::lookup_datastore(&store)?;

    let backup_group = BackupGroup::new(backup_type, backup_id);

    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;

    let user_info = CachedUserInfo::new()?;

    let privs = user_info.lookup_privs(&auth_id, &["datastore", &store]);

    let allowed = if (privs & PRIV_DATASTORE_MODIFY) != 0 {
        // High-privilege user/token
        true
    } else if (privs & PRIV_DATASTORE_BACKUP) != 0 {
        let owner = datastore.get_owner(&backup_group)?;

        match (owner.is_token(), new_owner.is_token()) {
            (true, true) => {
                // API token to API token, owned by same user
                let owner = owner.user();
                let new_owner = new_owner.user();
                owner == new_owner && Authid::from(owner.clone()) == auth_id
            },
            (true, false) => {
                // API token to API token owner
                Authid::from(owner.user().clone()) == auth_id
                    && new_owner == auth_id
            },
            (false, true) => {
                // API token owner to API token
                owner == auth_id
                    && Authid::from(new_owner.user().clone()) == auth_id
            },
            (false, false) => {
                // User to User, not allowed for unprivileged users
                false
            },
        }
    } else {
        false
    };

    if !allowed {
        return Err(http_err!(UNAUTHORIZED,
                  "{} does not have permission to change owner of backup group '{}' to {}",
                  auth_id,
                  backup_group,
                  new_owner,
        ));
    }

    if !user_info.is_active_auth_id(&new_owner) {
        bail!("{} '{}' is inactive or non-existent",
              if new_owner.is_token() {
                  "API token".to_string()
              } else {
                  "user".to_string()
              },
              new_owner);
    }

    datastore.set_owner(&backup_group, &new_owner, true)?;

    Ok(())
}

#[sortable]
const DATASTORE_INFO_SUBDIRS: SubdirMap = &[
    (
        "catalog",
        &Router::new()
            .get(&API_METHOD_CATALOG)
    ),
    (
        "change-owner",
        &Router::new()
            .post(&API_METHOD_SET_BACKUP_OWNER)
    ),
    (
        "download",
        &Router::new()
            .download(&API_METHOD_DOWNLOAD_FILE)
    ),
    (
        "download-decoded",
        &Router::new()
            .download(&API_METHOD_DOWNLOAD_FILE_DECODED)
    ),
    (
        "files",
        &Router::new()
            .get(&API_METHOD_LIST_SNAPSHOT_FILES)
    ),
    (
        "gc",
        &Router::new()
            .get(&API_METHOD_GARBAGE_COLLECTION_STATUS)
            .post(&API_METHOD_START_GARBAGE_COLLECTION)
    ),
    (
        "group-notes",
        &Router::new()
            .get(&API_METHOD_GET_GROUP_NOTES)
            .put(&API_METHOD_SET_GROUP_NOTES)
    ),
    (
        "groups",
        &Router::new()
            .get(&API_METHOD_LIST_GROUPS)
            .delete(&API_METHOD_DELETE_GROUP)
    ),
    (
        "notes",
        &Router::new()
            .get(&API_METHOD_GET_NOTES)
            .put(&API_METHOD_SET_NOTES)
    ),
    (
        "prune",
        &Router::new()
            .post(&API_METHOD_PRUNE)
    ),
    (
        "prune-datastore",
        &Router::new()
            .post(&API_METHOD_PRUNE_DATASTORE)
    ),
    (
        "pxar-file-download",
        &Router::new()
            .download(&API_METHOD_PXAR_FILE_DOWNLOAD)
    ),
    (
        "rrd",
        &Router::new()
            .get(&API_METHOD_GET_RRD_STATS)
    ),
    (
        "snapshots",
        &Router::new()
            .get(&API_METHOD_LIST_SNAPSHOTS)
            .delete(&API_METHOD_DELETE_SNAPSHOT)
    ),
    (
        "status",
        &Router::new()
            .get(&API_METHOD_STATUS)
    ),
    (
        "upload-backup-log",
        &Router::new()
            .upload(&API_METHOD_UPLOAD_BACKUP_LOG)
    ),
    (
        "verify",
        &Router::new()
            .post(&API_METHOD_VERIFY)
    ),
];

const DATASTORE_INFO_ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(DATASTORE_INFO_SUBDIRS))
    .subdirs(DATASTORE_INFO_SUBDIRS);


pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_GET_DATASTORE_LIST)
    .match_all("store", &DATASTORE_INFO_ROUTER);
