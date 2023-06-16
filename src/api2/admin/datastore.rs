//! Datastore Management

use std::collections::HashSet;
use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{bail, format_err, Error};
use futures::*;
use hyper::http::request::Parts;
use hyper::{header, Body, Response, StatusCode};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio_stream::wrappers::ReceiverStream;

use proxmox_async::blocking::WrappedReaderStream;
use proxmox_async::{io::AsyncChannelWriter, stream::AsyncReaderStream};
use proxmox_compression::zstd::ZstdEncoder;
use proxmox_router::{
    http_err, list_subdirs_api_method, ApiHandler, ApiMethod, ApiResponseFuture, Permission,
    Router, RpcEnvironment, RpcEnvironmentType, SubdirMap,
};
use proxmox_schema::*;
use proxmox_sortable_macro::sortable;
use proxmox_sys::fs::{
    file_read_firstline, file_read_optional_string, replace_file, CreateOptions,
};
use proxmox_sys::{task_log, task_warn};

use pxar::accessor::aio::Accessor;
use pxar::EntryKind;

use pbs_api_types::{
    print_ns_and_snapshot, print_store_and_ns, Authid, BackupContent, BackupNamespace, BackupType,
    Counts, CryptMode, DataStoreListItem, DataStoreStatus, GarbageCollectionStatus, GroupListItem,
    KeepOptions, Operation, PruneJobOptions, RRDMode, RRDTimeFrame, SnapshotListItem,
    SnapshotVerifyState, BACKUP_ARCHIVE_NAME_SCHEMA, BACKUP_ID_SCHEMA, BACKUP_NAMESPACE_SCHEMA,
    BACKUP_TIME_SCHEMA, BACKUP_TYPE_SCHEMA, DATASTORE_SCHEMA, IGNORE_VERIFIED_BACKUPS_SCHEMA,
    MAX_NAMESPACE_DEPTH, NS_MAX_DEPTH_SCHEMA, PRIV_DATASTORE_AUDIT, PRIV_DATASTORE_BACKUP,
    PRIV_DATASTORE_MODIFY, PRIV_DATASTORE_PRUNE, PRIV_DATASTORE_READ, PRIV_DATASTORE_VERIFY,
    UPID_SCHEMA, VERIFICATION_OUTDATED_AFTER_SCHEMA,
};
use pbs_client::pxar::{create_tar, create_zip};
use pbs_config::CachedUserInfo;
use pbs_datastore::backup_info::BackupInfo;
use pbs_datastore::cached_chunk_reader::CachedChunkReader;
use pbs_datastore::catalog::{ArchiveEntry, CatalogReader};
use pbs_datastore::data_blob::DataBlob;
use pbs_datastore::data_blob_reader::DataBlobReader;
use pbs_datastore::dynamic_index::{BufferedDynamicReader, DynamicIndexReader, LocalDynamicReadAt};
use pbs_datastore::fixed_index::FixedIndexReader;
use pbs_datastore::index::IndexFile;
use pbs_datastore::manifest::{BackupManifest, CLIENT_LOG_BLOB_NAME, MANIFEST_BLOB_NAME};
use pbs_datastore::prune::compute_prune_info;
use pbs_datastore::{
    check_backup_owner, task_tracking, BackupDir, BackupGroup, DataStore, LocalChunkReader,
    StoreProgress, CATALOG_NAME,
};
use pbs_tools::json::required_string_param;
use proxmox_rest_server::{formatter, WorkerTask};

use crate::api2::backup::optional_ns_param;
use crate::api2::node::rrd::create_value_from_rrd;
use crate::backup::{
    check_ns_privs_full, verify_all_backups, verify_backup_dir, verify_backup_group, verify_filter,
    ListAccessibleBackupGroups, NS_PRIVS_OK,
};

use crate::server::jobstate::Job;

const GROUP_NOTES_FILE_NAME: &str = "notes";

fn get_group_note_path(
    store: &DataStore,
    ns: &BackupNamespace,
    group: &pbs_api_types::BackupGroup,
) -> PathBuf {
    let mut note_path = store.group_path(ns, group);
    note_path.push(GROUP_NOTES_FILE_NAME);
    note_path
}

// helper to unify common sequence of checks:
// 1. check privs on NS (full or limited access)
// 2. load datastore
// 3. if needed (only limited access), check owner of group
fn check_privs_and_load_store(
    store: &str,
    ns: &BackupNamespace,
    auth_id: &Authid,
    full_access_privs: u64,
    partial_access_privs: u64,
    operation: Option<Operation>,
    backup_group: &pbs_api_types::BackupGroup,
) -> Result<Arc<DataStore>, Error> {
    let limited = check_ns_privs_full(store, ns, auth_id, full_access_privs, partial_access_privs)?;

    let datastore = DataStore::lookup_datastore(store, operation)?;

    if limited {
        let owner = datastore.get_owner(ns, backup_group)?;
        check_backup_owner(&owner, auth_id)?;
    }

    Ok(datastore)
}

fn read_backup_index(
    backup_dir: &BackupDir,
) -> Result<(BackupManifest, Vec<BackupContent>), Error> {
    let (manifest, index_size) = backup_dir.load_manifest()?;

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
    info: &BackupInfo,
) -> Result<(BackupManifest, Vec<BackupContent>), Error> {
    let (manifest, mut files) = read_backup_index(&info.backup_dir)?;

    let file_set = files.iter().fold(HashSet::new(), |mut acc, item| {
        acc.insert(item.filename.clone());
        acc
    });

    for file in &info.files {
        if file_set.contains(file) {
            continue;
        }
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
            ns: {
                type: BackupNamespace,
                optional: true,
            },
        },
    },
    returns: pbs_api_types::ADMIN_DATASTORE_LIST_GROUPS_RETURN_TYPE,
    access: {
        permission: &Permission::Anybody,
        description: "Requires DATASTORE_AUDIT for all or DATASTORE_BACKUP for owned groups on \
            /datastore/{store}[/{namespace}]",
    },
)]
/// List backup groups.
pub fn list_groups(
    store: String,
    ns: Option<BackupNamespace>,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<GroupListItem>, Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let ns = ns.unwrap_or_default();

    let list_all = !check_ns_privs_full(
        &store,
        &ns,
        &auth_id,
        PRIV_DATASTORE_AUDIT,
        PRIV_DATASTORE_BACKUP,
    )?;

    let datastore = DataStore::lookup_datastore(&store, Some(Operation::Read))?;

    datastore
        .iter_backup_groups(ns.clone())? // FIXME: Namespaces and recursion parameters!
        .try_fold(Vec::new(), |mut group_info, group| {
            let group = group?;

            let owner = match datastore.get_owner(&ns, group.as_ref()) {
                Ok(auth_id) => auth_id,
                Err(err) => {
                    eprintln!(
                        "Failed to get owner of group '{}' in {} - {}",
                        group.group(),
                        print_store_and_ns(&store, &ns),
                        err
                    );
                    return Ok(group_info);
                }
            };
            if !list_all && check_backup_owner(&owner, &auth_id).is_err() {
                return Ok(group_info);
            }

            let snapshots = match group.list_backups() {
                Ok(snapshots) => snapshots,
                Err(_) => return Ok(group_info),
            };

            let backup_count: u64 = snapshots.len() as u64;
            if backup_count == 0 {
                return Ok(group_info);
            }

            let last_backup = snapshots
                .iter()
                .fold(&snapshots[0], |a, b| {
                    if a.is_finished() && a.backup_dir.backup_time() > b.backup_dir.backup_time() {
                        a
                    } else {
                        b
                    }
                })
                .to_owned();

            let note_path = get_group_note_path(&datastore, &ns, group.as_ref());
            let comment = file_read_firstline(note_path).ok();

            group_info.push(GroupListItem {
                backup: group.into(),
                last_backup: last_backup.backup_dir.backup_time(),
                owner: Some(owner),
                backup_count,
                files: last_backup.files,
                comment,
            });

            Ok(group_info)
        })
}

#[api(
    input: {
        properties: {
            store: { schema: DATASTORE_SCHEMA },
            ns: {
                type: BackupNamespace,
                optional: true,
            },
            group: {
                type: pbs_api_types::BackupGroup,
                flatten: true,
            },
        },
    },
    access: {
        permission: &Permission::Anybody,
        description: "Requires on /datastore/{store}[/{namespace}] either DATASTORE_MODIFY for any\
            or DATASTORE_PRUNE and being the owner of the group",
    },
)]
/// Delete backup group including all snapshots.
pub async fn delete_group(
    store: String,
    ns: Option<BackupNamespace>,
    group: pbs_api_types::BackupGroup,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;

    tokio::task::spawn_blocking(move || {
        let ns = ns.unwrap_or_default();

        let datastore = check_privs_and_load_store(
            &store,
            &ns,
            &auth_id,
            PRIV_DATASTORE_MODIFY,
            PRIV_DATASTORE_PRUNE,
            Some(Operation::Write),
            &group,
        )?;

        if !datastore.remove_backup_group(&ns, &group)? {
            bail!("group only partially deleted due to protected snapshots");
        }

        Ok(Value::Null)
    })
    .await?
}

#[api(
    input: {
        properties: {
            store: { schema: DATASTORE_SCHEMA },
            ns: {
                type: BackupNamespace,
                optional: true,
            },
            backup_dir: {
                type: pbs_api_types::BackupDir,
                flatten: true,
            },
        },
    },
    returns: pbs_api_types::ADMIN_DATASTORE_LIST_SNAPSHOT_FILES_RETURN_TYPE,
    access: {
        permission: &Permission::Anybody,
        description: "Requires on /datastore/{store}[/{namespace}] either DATASTORE_AUDIT or \
            DATASTORE_READ for any or DATASTORE_BACKUP and being the owner of the group",
    },
)]
/// List snapshot files.
pub async fn list_snapshot_files(
    store: String,
    ns: Option<BackupNamespace>,
    backup_dir: pbs_api_types::BackupDir,
    _info: &ApiMethod,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<BackupContent>, Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;

    tokio::task::spawn_blocking(move || {
        let ns = ns.unwrap_or_default();

        let datastore = check_privs_and_load_store(
            &store,
            &ns,
            &auth_id,
            PRIV_DATASTORE_AUDIT | PRIV_DATASTORE_READ,
            PRIV_DATASTORE_BACKUP,
            Some(Operation::Read),
            &backup_dir.group,
        )?;

        let snapshot = datastore.backup_dir(ns, backup_dir)?;

        let info = BackupInfo::new(snapshot)?;

        let (_manifest, files) = get_all_snapshot_files(&info)?;

        Ok(files)
    })
    .await?
}

#[api(
    input: {
        properties: {
            store: { schema: DATASTORE_SCHEMA },
            ns: {
                type: BackupNamespace,
                optional: true,
            },
            backup_dir: {
                type: pbs_api_types::BackupDir,
                flatten: true,
            },
        },
    },
    access: {
        permission: &Permission::Anybody,
        description: "Requires on /datastore/{store}[/{namespace}] either DATASTORE_MODIFY for any\
            or DATASTORE_PRUNE and being the owner of the group",
    },
)]
/// Delete backup snapshot.
pub async fn delete_snapshot(
    store: String,
    ns: Option<BackupNamespace>,
    backup_dir: pbs_api_types::BackupDir,
    _info: &ApiMethod,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;

    tokio::task::spawn_blocking(move || {
        let ns = ns.unwrap_or_default();

        let datastore = check_privs_and_load_store(
            &store,
            &ns,
            &auth_id,
            PRIV_DATASTORE_MODIFY,
            PRIV_DATASTORE_PRUNE,
            Some(Operation::Write),
            &backup_dir.group,
        )?;

        let snapshot = datastore.backup_dir(ns, backup_dir)?;

        snapshot.destroy(false)?;

        Ok(Value::Null)
    })
    .await?
}

#[api(
    streaming: true,
    input: {
        properties: {
            store: { schema: DATASTORE_SCHEMA },
            ns: {
                type: BackupNamespace,
                optional: true,
            },
            "backup-type": {
                optional: true,
                type: BackupType,
            },
            "backup-id": {
                optional: true,
                schema: BACKUP_ID_SCHEMA,
            },
        },
    },
    returns: pbs_api_types::ADMIN_DATASTORE_LIST_SNAPSHOTS_RETURN_TYPE,
    access: {
        permission: &Permission::Anybody,
        description: "Requires on /datastore/{store}[/{namespace}] either DATASTORE_AUDIT for any \
            or DATASTORE_BACKUP and being the owner of the group",
    },
)]
/// List backup snapshots.
pub async fn list_snapshots(
    store: String,
    ns: Option<BackupNamespace>,
    backup_type: Option<BackupType>,
    backup_id: Option<String>,
    _param: Value,
    _info: &ApiMethod,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<SnapshotListItem>, Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;

    tokio::task::spawn_blocking(move || unsafe {
        list_snapshots_blocking(store, ns, backup_type, backup_id, auth_id)
    })
    .await
    .map_err(|err| format_err!("failed to await blocking task: {err}"))?
}

/// This must not run in a main worker thread as it potentially does tons of I/O.
unsafe fn list_snapshots_blocking(
    store: String,
    ns: Option<BackupNamespace>,
    backup_type: Option<BackupType>,
    backup_id: Option<String>,
    auth_id: Authid,
) -> Result<Vec<SnapshotListItem>, Error> {
    let ns = ns.unwrap_or_default();

    let list_all = !check_ns_privs_full(
        &store,
        &ns,
        &auth_id,
        PRIV_DATASTORE_AUDIT,
        PRIV_DATASTORE_BACKUP,
    )?;

    let datastore = DataStore::lookup_datastore(&store, Some(Operation::Read))?;

    // FIXME: filter also owner before collecting, for doing that nicely the owner should move into
    // backup group and provide an error free (Err -> None) accessor
    let groups = match (backup_type, backup_id) {
        (Some(backup_type), Some(backup_id)) => {
            vec![datastore.backup_group_from_parts(ns.clone(), backup_type, backup_id)]
        }
        // FIXME: Recursion
        (Some(backup_type), None) => datastore
            .iter_backup_type_ok(ns.clone(), backup_type)?
            .collect(),
        // FIXME: Recursion
        (None, Some(backup_id)) => BackupType::iter()
            .filter_map(|backup_type| {
                let group =
                    datastore.backup_group_from_parts(ns.clone(), backup_type, backup_id.clone());
                group.exists().then_some(group)
            })
            .collect(),
        // FIXME: Recursion
        (None, None) => datastore.list_backup_groups(ns.clone())?,
    };

    let info_to_snapshot_list_item = |group: &BackupGroup, owner, info: BackupInfo| {
        let backup = pbs_api_types::BackupDir {
            group: group.into(),
            time: info.backup_dir.backup_time(),
        };
        let protected = info.backup_dir.is_protected();

        match get_all_snapshot_files(&info) {
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
                    }
                };

                let verification = manifest.unprotected["verify_state"].clone();
                let verification: Option<SnapshotVerifyState> =
                    match serde_json::from_value(verification) {
                        Ok(verify) => verify,
                        Err(err) => {
                            eprintln!("error parsing verification state : '{}'", err);
                            None
                        }
                    };

                let size = Some(files.iter().map(|x| x.size.unwrap_or(0)).sum());

                SnapshotListItem {
                    backup,
                    comment,
                    verification,
                    fingerprint,
                    files,
                    size,
                    owner,
                    protected,
                }
            }
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
                    backup,
                    comment: None,
                    verification: None,
                    fingerprint: None,
                    files,
                    size: None,
                    owner,
                    protected,
                }
            }
        }
    };

    groups.iter().try_fold(Vec::new(), |mut snapshots, group| {
        let owner = match group.get_owner() {
            Ok(auth_id) => auth_id,
            Err(err) => {
                eprintln!(
                    "Failed to get owner of group '{}' in {} - {}",
                    group.group(),
                    print_store_and_ns(&store, &ns),
                    err
                );
                return Ok(snapshots);
            }
        };

        if !list_all && check_backup_owner(&owner, &auth_id).is_err() {
            return Ok(snapshots);
        }

        let group_backups = group.list_backups()?;

        snapshots.extend(
            group_backups
                .into_iter()
                .map(|info| info_to_snapshot_list_item(group, Some(owner.clone()), info)),
        );

        Ok(snapshots)
    })
}

async fn get_snapshots_count(
    store: &Arc<DataStore>,
    owner: Option<&Authid>,
) -> Result<Counts, Error> {
    let store = Arc::clone(store);
    let owner = owner.cloned();
    tokio::task::spawn_blocking(move || {
        let root_ns = Default::default();
        ListAccessibleBackupGroups::new_with_privs(
            &store,
            root_ns,
            MAX_NAMESPACE_DEPTH,
            Some(PRIV_DATASTORE_AUDIT | PRIV_DATASTORE_READ),
            None,
            owner.as_ref(),
        )?
        .try_fold(Counts::default(), |mut counts, group| {
            let group = match group {
                Ok(group) => group,
                Err(_) => return Ok(counts), // TODO: add this as error counts?
            };
            let snapshot_count = group.list_backups()?.len() as u64;

            // only include groups with snapshots, counting/displaying empty groups can confuse
            if snapshot_count > 0 {
                let type_count = match group.backup_type() {
                    BackupType::Ct => counts.ct.get_or_insert(Default::default()),
                    BackupType::Vm => counts.vm.get_or_insert(Default::default()),
                    BackupType::Host => counts.host.get_or_insert(Default::default()),
                };

                type_count.groups += 1;
                type_count.snapshots += snapshot_count;
            }

            Ok(counts)
        })
    })
    .await?
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
        permission: &Permission::Anybody,
        description: "Requires on /datastore/{store} either DATASTORE_AUDIT or DATASTORE_BACKUP for \
            the full statistics. Counts of accessible groups are always returned, if any",
    },
)]
/// Get datastore status.
pub async fn status(
    store: String,
    verbose: bool,
    _info: &ApiMethod,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<DataStoreStatus, Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let user_info = CachedUserInfo::new()?;
    let store_privs = user_info.lookup_privs(&auth_id, &["datastore", &store]);

    let datastore = DataStore::lookup_datastore(&store, Some(Operation::Read));

    let store_stats = if store_privs & (PRIV_DATASTORE_AUDIT | PRIV_DATASTORE_BACKUP) != 0 {
        true
    } else if store_privs & PRIV_DATASTORE_READ != 0 {
        false // allow at least counts, user can read groups anyway..
    } else {
        match user_info.any_privs_below(&auth_id, &["datastore", &store], NS_PRIVS_OK) {
            // avoid leaking existence info if users hasn't at least any priv. below
            Ok(false) | Err(_) => return Err(http_err!(FORBIDDEN, "permission check failed")),
            _ => false,
        }
    };
    let datastore = datastore?; // only unwrap no to avoid leaking existence info

    let (counts, gc_status) = if verbose {
        let filter_owner = if store_privs & PRIV_DATASTORE_AUDIT != 0 {
            None
        } else {
            Some(&auth_id)
        };

        let counts = Some(get_snapshots_count(&datastore, filter_owner).await?);
        let gc_status = if store_stats {
            Some(datastore.last_gc_status())
        } else {
            None
        };

        (counts, gc_status)
    } else {
        (None, None)
    };

    Ok(if store_stats {
        let storage = crate::tools::fs::fs_info(datastore.base_path()).await?;
        DataStoreStatus {
            total: storage.total,
            used: storage.used,
            avail: storage.available,
            gc_status,
            counts,
        }
    } else {
        DataStoreStatus {
            total: 0,
            used: 0,
            avail: 0,
            gc_status,
            counts,
        }
    })
}

#[api(
    input: {
        properties: {
            store: {
                schema: DATASTORE_SCHEMA,
            },
            ns: {
                type: BackupNamespace,
                optional: true,
            },
            "backup-type": {
                type: BackupType,
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
            "max-depth": {
                schema: NS_MAX_DEPTH_SCHEMA,
                optional: true,
            },
        },
    },
    returns: {
        schema: UPID_SCHEMA,
    },
    access: {
        permission: &Permission::Anybody,
        description: "Requires on /datastore/{store}[/{namespace}] either DATASTORE_VERIFY for any \
            or DATASTORE_BACKUP and being the owner of the group",
    },
)]
/// Verify backups.
///
/// This function can verify a single backup snapshot, all backup from a backup group,
/// or all backups in the datastore.
#[allow(clippy::too_many_arguments)]
pub fn verify(
    store: String,
    ns: Option<BackupNamespace>,
    backup_type: Option<BackupType>,
    backup_id: Option<String>,
    backup_time: Option<i64>,
    ignore_verified: Option<bool>,
    outdated_after: Option<i64>,
    max_depth: Option<usize>,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let ns = ns.unwrap_or_default();

    let owner_check_required = check_ns_privs_full(
        &store,
        &ns,
        &auth_id,
        PRIV_DATASTORE_VERIFY,
        PRIV_DATASTORE_BACKUP,
    )?;

    let datastore = DataStore::lookup_datastore(&store, Some(Operation::Read))?;
    let ignore_verified = ignore_verified.unwrap_or(true);

    let worker_id;

    let mut backup_dir = None;
    let mut backup_group = None;
    let mut worker_type = "verify";

    match (backup_type, backup_id, backup_time) {
        (Some(backup_type), Some(backup_id), Some(backup_time)) => {
            worker_id = format!(
                "{}:{}/{}/{}/{:08X}",
                store,
                ns.display_as_path(),
                backup_type,
                backup_id,
                backup_time
            );
            let dir =
                datastore.backup_dir_from_parts(ns.clone(), backup_type, backup_id, backup_time)?;

            if owner_check_required {
                let owner = datastore.get_owner(dir.backup_ns(), dir.as_ref())?;
                check_backup_owner(&owner, &auth_id)?;
            }

            backup_dir = Some(dir);
            worker_type = "verify_snapshot";
        }
        (Some(backup_type), Some(backup_id), None) => {
            worker_id = format!(
                "{}:{}/{}/{}",
                store,
                ns.display_as_path(),
                backup_type,
                backup_id
            );
            let group = pbs_api_types::BackupGroup::from((backup_type, backup_id));

            if owner_check_required {
                let owner = datastore.get_owner(&ns, &group)?;
                check_backup_owner(&owner, &auth_id)?;
            }

            backup_group = Some(datastore.backup_group(ns.clone(), group));
            worker_type = "verify_group";
        }
        (None, None, None) => {
            worker_id = if ns.is_root() {
                store
            } else {
                format!("{}:{}", store, ns.display_as_path())
            };
        }
        _ => bail!("parameters do not specify a backup group or snapshot"),
    }

    let to_stdout = rpcenv.env_type() == RpcEnvironmentType::CLI;

    let upid_str = WorkerTask::new_thread(
        worker_type,
        Some(worker_id),
        auth_id.to_string(),
        to_stdout,
        move |worker| {
            let verify_worker = crate::backup::VerifyWorker::new(worker.clone(), datastore);
            let failed_dirs = if let Some(backup_dir) = backup_dir {
                let mut res = Vec::new();
                if !verify_backup_dir(
                    &verify_worker,
                    &backup_dir,
                    worker.upid().clone(),
                    Some(&move |manifest| verify_filter(ignore_verified, outdated_after, manifest)),
                )? {
                    res.push(print_ns_and_snapshot(
                        backup_dir.backup_ns(),
                        backup_dir.as_ref(),
                    ));
                }
                res
            } else if let Some(backup_group) = backup_group {
                verify_backup_group(
                    &verify_worker,
                    &backup_group,
                    &mut StoreProgress::new(1),
                    worker.upid(),
                    Some(&move |manifest| verify_filter(ignore_verified, outdated_after, manifest)),
                )?
            } else {
                let owner = if owner_check_required {
                    Some(&auth_id)
                } else {
                    None
                };

                verify_all_backups(
                    &verify_worker,
                    worker.upid(),
                    ns,
                    max_depth,
                    owner,
                    Some(&move |manifest| verify_filter(ignore_verified, outdated_after, manifest)),
                )?
            };
            if !failed_dirs.is_empty() {
                task_log!(worker, "Failed to verify the following snapshots/groups:");
                for dir in failed_dirs {
                    task_log!(worker, "\t{}", dir);
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
            group: {
                type: pbs_api_types::BackupGroup,
                flatten: true,
            },
            "dry-run": {
                optional: true,
                type: bool,
                default: false,
                description: "Just show what prune would do, but do not delete anything.",
            },
            "keep-options": {
                type: KeepOptions,
                flatten: true,
            },
            store: {
                schema: DATASTORE_SCHEMA,
            },
            ns: {
                type: BackupNamespace,
                optional: true,
            },
        },
    },
    returns: pbs_api_types::ADMIN_DATASTORE_PRUNE_RETURN_TYPE,
    access: {
        permission: &Permission::Anybody,
        description: "Requires on /datastore/{store}[/{namespace}] either DATASTORE_MODIFY for any\
            or DATASTORE_PRUNE and being the owner of the group",
    },
)]
/// Prune a group on the datastore
pub fn prune(
    group: pbs_api_types::BackupGroup,
    dry_run: bool,
    keep_options: KeepOptions,
    store: String,
    ns: Option<BackupNamespace>,
    _param: Value,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let ns = ns.unwrap_or_default();
    let datastore = check_privs_and_load_store(
        &store,
        &ns,
        &auth_id,
        PRIV_DATASTORE_MODIFY,
        PRIV_DATASTORE_PRUNE,
        Some(Operation::Write),
        &group,
    )?;

    let worker_id = format!("{}:{}:{}", store, ns, group);
    let group = datastore.backup_group(ns.clone(), group);

    let mut prune_result = Vec::new();

    let list = group.list_backups()?;

    let mut prune_info = compute_prune_info(list, &keep_options)?;

    prune_info.reverse(); // delete older snapshots first

    let keep_all = !keep_options.keeps_something();

    if dry_run {
        for (info, mark) in prune_info {
            let keep = keep_all || mark.keep();

            let mut result = json!({
                "backup-type": info.backup_dir.backup_type(),
                "backup-id": info.backup_dir.backup_id(),
                "backup-time": info.backup_dir.backup_time(),
                "keep": keep,
                "protected": mark.protected(),
            });
            let prune_ns = info.backup_dir.backup_ns();
            if !prune_ns.is_root() {
                result["ns"] = serde_json::to_value(prune_ns)?;
            }
            prune_result.push(result);
        }
        return Ok(json!(prune_result));
    }

    // We use a WorkerTask just to have a task log, but run synchrounously
    let worker = WorkerTask::new("prune", Some(worker_id), auth_id.to_string(), true)?;

    if keep_all {
        task_log!(worker, "No prune selection - keeping all files.");
    } else {
        let mut opts = Vec::new();
        if !ns.is_root() {
            opts.push(format!("--ns {ns}"));
        }
        crate::server::cli_keep_options(&mut opts, &keep_options);

        task_log!(worker, "retention options: {}", opts.join(" "));
        task_log!(
            worker,
            "Starting prune on {} group \"{}\"",
            print_store_and_ns(&store, &ns),
            group.group(),
        );
    }

    for (info, mark) in prune_info {
        let keep = keep_all || mark.keep();

        let backup_time = info.backup_dir.backup_time();
        let timestamp = info.backup_dir.backup_time_string();
        let group: &pbs_api_types::BackupGroup = info.backup_dir.as_ref();

        let msg = format!("{}/{}/{} {}", group.ty, group.id, timestamp, mark,);

        task_log!(worker, "{}", msg);

        prune_result.push(json!({
            "backup-type": group.ty,
            "backup-id": group.id,
            "backup-time": backup_time,
            "keep": keep,
            "protected": mark.protected(),
        }));

        if !(dry_run || keep) {
            if let Err(err) = info.backup_dir.destroy(false) {
                task_warn!(
                    worker,
                    "failed to remove dir {:?}: {}",
                    info.backup_dir.relative_path(),
                    err,
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
                type: PruneJobOptions,
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
        permission: &Permission::Anybody,
        description: "Requires Datastore.Modify or Datastore.Prune on the datastore/namespace.",
    },
)]
/// Prune the datastore
pub fn prune_datastore(
    dry_run: bool,
    prune_options: PruneJobOptions,
    store: String,
    _param: Value,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<String, Error> {
    let user_info = CachedUserInfo::new()?;

    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;

    user_info.check_privs(
        &auth_id,
        &prune_options.acl_path(&store),
        PRIV_DATASTORE_MODIFY | PRIV_DATASTORE_PRUNE,
        true,
    )?;

    let datastore = DataStore::lookup_datastore(&store, Some(Operation::Write))?;
    let ns = prune_options.ns.clone().unwrap_or_default();
    let worker_id = format!("{}:{}", store, ns);

    let to_stdout = rpcenv.env_type() == RpcEnvironmentType::CLI;

    let upid_str = WorkerTask::new_thread(
        "prune",
        Some(worker_id),
        auth_id.to_string(),
        to_stdout,
        move |worker| {
            crate::server::prune_datastore(worker, auth_id, prune_options, datastore, dry_run)
        },
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
    let datastore = DataStore::lookup_datastore(&store, Some(Operation::Write))?;
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;

    let job = Job::new("garbage_collection", &store)
        .map_err(|_| format_err!("garbage collection already running"))?;

    let to_stdout = rpcenv.env_type() == RpcEnvironmentType::CLI;

    let upid_str =
        crate::server::do_garbage_collection_job(job, datastore, &auth_id, None, to_stdout)
            .map_err(|err| {
                format_err!(
                    "unable to start garbage collection job on datastore {} - {}",
                    store,
                    err
                )
            })?;

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
    let datastore = DataStore::lookup_datastore(&store, Some(Operation::Read))?;

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
        let acl_path = &["datastore", store];
        let user_privs = user_info.lookup_privs(&auth_id, acl_path);
        let allowed = (user_privs & (PRIV_DATASTORE_AUDIT | PRIV_DATASTORE_BACKUP)) != 0;

        let mut allow_id = false;
        if !allowed {
            if let Ok(any_privs) = user_info.any_privs_below(&auth_id, acl_path, NS_PRIVS_OK) {
                allow_id = any_privs;
            }
        }

        if allowed || allow_id {
            list.push(DataStoreListItem {
                store: store.clone(),
                comment: if !allowed {
                    None
                } else {
                    data["comment"].as_str().map(String::from)
                },
                maintenance: data["maintenance-mode"].as_str().map(String::from),
            });
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
            ("ns", true, &BACKUP_NAMESPACE_SCHEMA),
            ("backup-type", false, &BACKUP_TYPE_SCHEMA),
            ("backup-id", false, &BACKUP_ID_SCHEMA),
            ("backup-time", false, &BACKUP_TIME_SCHEMA),
            ("file-name", false, &BACKUP_ARCHIVE_NAME_SCHEMA),
        ]),
    ),
)
.access(
    Some(
        "Requires on /datastore/{store}[/{namespace}] either DATASTORE_READ for any or \
        DATASTORE_BACKUP and being the owner of the group",
    ),
    &Permission::Anybody,
);

pub fn download_file(
    _parts: Parts,
    _req_body: Body,
    param: Value,
    _info: &ApiMethod,
    rpcenv: Box<dyn RpcEnvironment>,
) -> ApiResponseFuture {
    async move {
        let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
        let store = required_string_param(&param, "store")?;
        let backup_ns = optional_ns_param(&param)?;

        let backup_dir: pbs_api_types::BackupDir = Deserialize::deserialize(&param)?;
        let datastore = check_privs_and_load_store(
            store,
            &backup_ns,
            &auth_id,
            PRIV_DATASTORE_READ,
            PRIV_DATASTORE_BACKUP,
            Some(Operation::Read),
            &backup_dir.group,
        )?;

        let file_name = required_string_param(&param, "file-name")?.to_owned();

        println!(
            "Download {} from {} ({}/{})",
            file_name,
            print_store_and_ns(store, &backup_ns),
            backup_dir,
            file_name
        );

        let backup_dir = datastore.backup_dir(backup_ns, backup_dir)?;

        let mut path = datastore.base_path();
        path.push(backup_dir.relative_path());
        path.push(&file_name);

        let file = tokio::fs::File::open(&path)
            .await
            .map_err(|err| http_err!(BAD_REQUEST, "File open failed: {}", err))?;

        let payload =
            tokio_util::codec::FramedRead::new(file, tokio_util::codec::BytesCodec::new())
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
    }
    .boxed()
}

#[sortable]
pub const API_METHOD_DOWNLOAD_FILE_DECODED: ApiMethod = ApiMethod::new(
    &ApiHandler::AsyncHttp(&download_file_decoded),
    &ObjectSchema::new(
        "Download single decoded file from backup snapshot. Only works if it's not encrypted.",
        &sorted!([
            ("store", false, &DATASTORE_SCHEMA),
            ("ns", true, &BACKUP_NAMESPACE_SCHEMA),
            ("backup-type", false, &BACKUP_TYPE_SCHEMA),
            ("backup-id", false, &BACKUP_ID_SCHEMA),
            ("backup-time", false, &BACKUP_TIME_SCHEMA),
            ("file-name", false, &BACKUP_ARCHIVE_NAME_SCHEMA),
        ]),
    ),
)
.access(
    Some(
        "Requires on /datastore/{store}[/{namespace}] either DATASTORE_READ for any or \
        DATASTORE_BACKUP and being the owner of the group",
    ),
    &Permission::Anybody,
);

pub fn download_file_decoded(
    _parts: Parts,
    _req_body: Body,
    param: Value,
    _info: &ApiMethod,
    rpcenv: Box<dyn RpcEnvironment>,
) -> ApiResponseFuture {
    async move {
        let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
        let store = required_string_param(&param, "store")?;
        let backup_ns = optional_ns_param(&param)?;

        let backup_dir_api: pbs_api_types::BackupDir = Deserialize::deserialize(&param)?;
        let datastore = check_privs_and_load_store(
            store,
            &backup_ns,
            &auth_id,
            PRIV_DATASTORE_READ,
            PRIV_DATASTORE_BACKUP,
            Some(Operation::Read),
            &backup_dir_api.group,
        )?;

        let file_name = required_string_param(&param, "file-name")?.to_owned();
        let backup_dir = datastore.backup_dir(backup_ns.clone(), backup_dir_api.clone())?;

        let (manifest, files) = read_backup_index(&backup_dir)?;
        for file in files {
            if file.filename == file_name && file.crypt_mode == Some(CryptMode::Encrypt) {
                bail!("cannot decode '{}' - is encrypted", file_name);
            }
        }

        println!(
            "Download {} from {} ({}/{})",
            file_name,
            print_store_and_ns(store, &backup_ns),
            backup_dir_api,
            file_name
        );

        let mut path = datastore.base_path();
        path.push(backup_dir.relative_path());
        path.push(&file_name);

        let (_, extension) = file_name.rsplit_once('.').unwrap();

        let body = match extension {
            "didx" => {
                let index = DynamicIndexReader::open(&path).map_err(|err| {
                    format_err!("unable to read dynamic index '{:?}' - {}", &path, err)
                })?;
                let (csum, size) = index.compute_csum();
                manifest.verify_file(&file_name, &csum, size)?;

                let chunk_reader = LocalChunkReader::new(datastore, None, CryptMode::None);
                let reader = CachedChunkReader::new(chunk_reader, index, 1).seekable();
                Body::wrap_stream(AsyncReaderStream::new(reader).map_err(move |err| {
                    eprintln!("error during streaming of '{:?}' - {}", path, err);
                    err
                }))
            }
            "fidx" => {
                let index = FixedIndexReader::open(&path).map_err(|err| {
                    format_err!("unable to read fixed index '{:?}' - {}", &path, err)
                })?;

                let (csum, size) = index.compute_csum();
                manifest.verify_file(&file_name, &csum, size)?;

                let chunk_reader = LocalChunkReader::new(datastore, None, CryptMode::None);
                let reader = CachedChunkReader::new(chunk_reader, index, 1).seekable();
                Body::wrap_stream(
                    AsyncReaderStream::with_buffer_size(reader, 4 * 1024 * 1024).map_err(
                        move |err| {
                            eprintln!("error during streaming of '{:?}' - {}", path, err);
                            err
                        },
                    ),
                )
            }
            "blob" => {
                let file = std::fs::File::open(&path)
                    .map_err(|err| http_err!(BAD_REQUEST, "File open failed: {}", err))?;

                // FIXME: load full blob to verify index checksum?

                Body::wrap_stream(
                    WrappedReaderStream::new(DataBlobReader::new(file, None)?).map_err(
                        move |err| {
                            eprintln!("error during streaming of '{:?}' - {}", path, err);
                            err
                        },
                    ),
                )
            }
            extension => {
                bail!("cannot download '{}' files", extension);
            }
        };

        // fixme: set other headers ?
        Ok(Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/octet-stream")
            .body(body)
            .unwrap())
    }
    .boxed()
}

#[sortable]
pub const API_METHOD_UPLOAD_BACKUP_LOG: ApiMethod = ApiMethod::new(
    &ApiHandler::AsyncHttp(&upload_backup_log),
    &ObjectSchema::new(
        "Upload the client backup log file into a backup snapshot ('client.log.blob').",
        &sorted!([
            ("store", false, &DATASTORE_SCHEMA),
            ("ns", true, &BACKUP_NAMESPACE_SCHEMA),
            ("backup-type", false, &BACKUP_TYPE_SCHEMA),
            ("backup-id", false, &BACKUP_ID_SCHEMA),
            ("backup-time", false, &BACKUP_TIME_SCHEMA),
        ]),
    ),
)
.access(
    Some("Only the backup creator/owner is allowed to do this."),
    &Permission::Anybody,
);

pub fn upload_backup_log(
    _parts: Parts,
    req_body: Body,
    param: Value,
    _info: &ApiMethod,
    rpcenv: Box<dyn RpcEnvironment>,
) -> ApiResponseFuture {
    async move {
        let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
        let store = required_string_param(&param, "store")?;
        let backup_ns = optional_ns_param(&param)?;

        let backup_dir_api: pbs_api_types::BackupDir = Deserialize::deserialize(&param)?;

        let datastore = check_privs_and_load_store(
            store,
            &backup_ns,
            &auth_id,
            0,
            PRIV_DATASTORE_BACKUP,
            Some(Operation::Write),
            &backup_dir_api.group,
        )?;
        let backup_dir = datastore.backup_dir(backup_ns.clone(), backup_dir_api.clone())?;

        let file_name = CLIENT_LOG_BLOB_NAME;

        let mut path = backup_dir.full_path();
        path.push(file_name);

        if path.exists() {
            bail!("backup already contains a log.");
        }

        println!(
            "Upload backup log to {} {backup_dir_api}/{file_name}",
            print_store_and_ns(store, &backup_ns),
        );

        let data = req_body
            .map_err(Error::from)
            .try_fold(Vec::new(), |mut acc, chunk| {
                acc.extend_from_slice(&chunk);
                future::ok::<_, Error>(acc)
            })
            .await?;

        // always verify blob/CRC at server side
        let blob = DataBlob::load_from_reader(&mut &data[..])?;

        replace_file(&path, blob.raw_data(), CreateOptions::new(), false)?;

        // fixme: use correct formatter
        Ok(formatter::JSON_FORMATTER.format_data(Value::Null, &*rpcenv))
    }
    .boxed()
}

#[api(
    input: {
        properties: {
            store: { schema: DATASTORE_SCHEMA },
            ns: {
                type: BackupNamespace,
                optional: true,
            },
            backup_dir: {
                type: pbs_api_types::BackupDir,
                flatten: true,
            },
            "filepath": {
                description: "Base64 encoded path.",
                type: String,
            }
        },
    },
    access: {
        description: "Requires on /datastore/{store}[/{namespace}] either DATASTORE_READ for any or \
            DATASTORE_BACKUP and being the owner of the group",
        permission: &Permission::Anybody,
    },
)]
/// Get the entries of the given path of the catalog
pub async fn catalog(
    store: String,
    ns: Option<BackupNamespace>,
    backup_dir: pbs_api_types::BackupDir,
    filepath: String,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<ArchiveEntry>, Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;

    tokio::task::spawn_blocking(move || {
        let ns = ns.unwrap_or_default();

        let datastore = check_privs_and_load_store(
            &store,
            &ns,
            &auth_id,
            PRIV_DATASTORE_READ,
            PRIV_DATASTORE_BACKUP,
            Some(Operation::Read),
            &backup_dir.group,
        )?;

        let backup_dir = datastore.backup_dir(ns, backup_dir)?;

        let file_name = CATALOG_NAME;

        let (manifest, files) = read_backup_index(&backup_dir)?;
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
        manifest.verify_file(file_name, &csum, size)?;

        let chunk_reader = LocalChunkReader::new(datastore, None, CryptMode::None);
        let reader = BufferedDynamicReader::new(index, chunk_reader);

        let mut catalog_reader = CatalogReader::new(reader);

        let path = if filepath != "root" && filepath != "/" {
            base64::decode(filepath)?
        } else {
            vec![b'/']
        };

        catalog_reader.list_dir_contents(&path)
    })
    .await?
}

#[sortable]
pub const API_METHOD_PXAR_FILE_DOWNLOAD: ApiMethod = ApiMethod::new(
    &ApiHandler::AsyncHttp(&pxar_file_download),
    &ObjectSchema::new(
        "Download single file from pxar file of a backup snapshot. Only works if it's not encrypted.",
        &sorted!([
            ("store", false, &DATASTORE_SCHEMA),
            ("ns", true, &BACKUP_NAMESPACE_SCHEMA),
            ("backup-type", false, &BACKUP_TYPE_SCHEMA),
            ("backup-id", false,  &BACKUP_ID_SCHEMA),
            ("backup-time", false, &BACKUP_TIME_SCHEMA),
            ("filepath", false, &StringSchema::new("Base64 encoded path").schema()),
            ("tar", true, &BooleanSchema::new("Download as .tar.zst").schema()),
        ]),
    )
).access(
    Some(
        "Requires on /datastore/{store}[/{namespace}] either DATASTORE_READ for any or \
        DATASTORE_BACKUP and being the owner of the group",
    ),
    &Permission::Anybody,
);

pub fn pxar_file_download(
    _parts: Parts,
    _req_body: Body,
    param: Value,
    _info: &ApiMethod,
    rpcenv: Box<dyn RpcEnvironment>,
) -> ApiResponseFuture {
    async move {
        let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
        let store = required_string_param(&param, "store")?;
        let ns = optional_ns_param(&param)?;

        let backup_dir: pbs_api_types::BackupDir = Deserialize::deserialize(&param)?;
        let datastore = check_privs_and_load_store(
            store,
            &ns,
            &auth_id,
            PRIV_DATASTORE_READ,
            PRIV_DATASTORE_BACKUP,
            Some(Operation::Read),
            &backup_dir.group,
        )?;

        let backup_dir = datastore.backup_dir(ns, backup_dir)?;

        let filepath = required_string_param(&param, "filepath")?.to_owned();

        let tar = param["tar"].as_bool().unwrap_or(false);

        let mut components = base64::decode(&filepath)?;
        if !components.is_empty() && components[0] == b'/' {
            components.remove(0);
        }

        let mut split = components.splitn(2, |c| *c == b'/');
        let pxar_name = std::str::from_utf8(split.next().unwrap())?;
        let file_path = split.next().unwrap_or(b"/");
        let (manifest, files) = read_backup_index(&backup_dir)?;
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
        manifest.verify_file(pxar_name, &csum, size)?;

        let chunk_reader = LocalChunkReader::new(datastore, None, CryptMode::None);
        let reader = BufferedDynamicReader::new(index, chunk_reader);
        let archive_size = reader.archive_size();
        let reader = LocalDynamicReadAt::new(reader);

        let decoder = Accessor::new(reader, archive_size).await?;
        let root = decoder.open_root().await?;
        let path = OsStr::from_bytes(file_path).to_os_string();
        let file = root
            .lookup(&path)
            .await?
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
                        eprintln!("error during streaming of hardlink '{:?}' - {}", path, err);
                        err
                    }),
            ),
            EntryKind::Directory => {
                let (sender, receiver) = tokio::sync::mpsc::channel::<Result<_, Error>>(100);
                let channelwriter = AsyncChannelWriter::new(sender, 1024 * 1024);
                if tar {
                    proxmox_rest_server::spawn_internal_task(create_tar(
                        channelwriter,
                        decoder,
                        path.clone(),
                    ));
                    let zstdstream = ZstdEncoder::new(ReceiverStream::new(receiver))?;
                    Body::wrap_stream(zstdstream.map_err(move |err| {
                        log::error!("error during streaming of tar.zst '{:?}' - {}", path, err);
                        err
                    }))
                } else {
                    proxmox_rest_server::spawn_internal_task(create_zip(
                        channelwriter,
                        decoder,
                        path.clone(),
                    ));
                    Body::wrap_stream(ReceiverStream::new(receiver).map_err(move |err| {
                        log::error!("error during streaming of zip '{:?}' - {}", path, err);
                        err
                    }))
                }
            }
            other => bail!("cannot download file of type {:?}", other),
        };

        // fixme: set other headers ?
        Ok(Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/octet-stream")
            .body(body)
            .unwrap())
    }
    .boxed()
}

#[api(
    input: {
        properties: {
            store: {
                schema: DATASTORE_SCHEMA,
            },
            timeframe: {
                type: RRDTimeFrame,
            },
            cf: {
                type: RRDMode,
            },
        },
    },
    access: {
        permission: &Permission::Privilege(
            &["datastore", "{store}"], PRIV_DATASTORE_AUDIT | PRIV_DATASTORE_BACKUP, true),
    },
)]
/// Read datastore stats
pub fn get_rrd_stats(
    store: String,
    timeframe: RRDTimeFrame,
    cf: RRDMode,
    _param: Value,
) -> Result<Value, Error> {
    let datastore = DataStore::lookup_datastore(&store, Some(Operation::Read))?;
    let disk_manager = crate::tools::disks::DiskManage::new();

    let mut rrd_fields = vec![
        "total",
        "available",
        "used",
        "read_ios",
        "read_bytes",
        "write_ios",
        "write_bytes",
    ];

    // we do not have io_ticks for zpools, so don't include them
    match disk_manager.find_mounted_device(&datastore.base_path()) {
        Ok(Some((fs_type, _, _))) if fs_type.as_str() == "zfs" => {}
        _ => rrd_fields.push("io_ticks"),
    };

    create_value_from_rrd(&format!("datastore/{}", store), &rrd_fields, timeframe, cf)
}

#[api(
    input: {
        properties: {
            store: {
                schema: DATASTORE_SCHEMA,
            },
        },
    },
    access: {
        permission: &Permission::Privilege(&["datastore", "{store}"], PRIV_DATASTORE_AUDIT, true),
    },
)]
/// Read datastore stats
pub fn get_active_operations(store: String, _param: Value) -> Result<Value, Error> {
    let active_operations = task_tracking::get_active_operations(&store)?;
    Ok(json!({
        "read": active_operations.read,
        "write": active_operations.write,
    }))
}

#[api(
    input: {
        properties: {
            store: { schema: DATASTORE_SCHEMA },
            ns: {
                type: BackupNamespace,
                optional: true,
            },
            backup_group: {
                type: pbs_api_types::BackupGroup,
                flatten: true,
            },
        },
    },
    access: {
        permission: &Permission::Anybody,
        description: "Requires on /datastore/{store}[/{namespace}] either DATASTORE_AUDIT for any \
            or DATASTORE_BACKUP and being the owner of the group",
    },
)]
/// Get "notes" for a backup group
pub fn get_group_notes(
    store: String,
    ns: Option<BackupNamespace>,
    backup_group: pbs_api_types::BackupGroup,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<String, Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let ns = ns.unwrap_or_default();

    let datastore = check_privs_and_load_store(
        &store,
        &ns,
        &auth_id,
        PRIV_DATASTORE_AUDIT,
        PRIV_DATASTORE_BACKUP,
        Some(Operation::Read),
        &backup_group,
    )?;

    let note_path = get_group_note_path(&datastore, &ns, &backup_group);
    Ok(file_read_optional_string(note_path)?.unwrap_or_else(|| "".to_owned()))
}

#[api(
    input: {
        properties: {
            store: { schema: DATASTORE_SCHEMA },
            ns: {
                type: BackupNamespace,
                optional: true,
            },
            backup_group: {
                type: pbs_api_types::BackupGroup,
                flatten: true,
            },
            notes: {
                description: "A multiline text.",
            },
        },
    },
    access: {
        permission: &Permission::Anybody,
        description: "Requires on /datastore/{store}[/{namespace}] either DATASTORE_MODIFY for any \
            or DATASTORE_BACKUP and being the owner of the group",
    },
)]
/// Set "notes" for a backup group
pub fn set_group_notes(
    store: String,
    ns: Option<BackupNamespace>,
    backup_group: pbs_api_types::BackupGroup,
    notes: String,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let ns = ns.unwrap_or_default();

    let datastore = check_privs_and_load_store(
        &store,
        &ns,
        &auth_id,
        PRIV_DATASTORE_MODIFY,
        PRIV_DATASTORE_BACKUP,
        Some(Operation::Write),
        &backup_group,
    )?;

    let note_path = get_group_note_path(&datastore, &ns, &backup_group);
    replace_file(note_path, notes.as_bytes(), CreateOptions::new(), false)?;

    Ok(())
}

#[api(
    input: {
        properties: {
            store: { schema: DATASTORE_SCHEMA },
            ns: {
                type: BackupNamespace,
                optional: true,
            },
            backup_dir: {
                type: pbs_api_types::BackupDir,
                flatten: true,
            },
        },
    },
    access: {
        permission: &Permission::Anybody,
        description: "Requires on /datastore/{store}[/{namespace}] either DATASTORE_AUDIT for any \
            or DATASTORE_BACKUP and being the owner of the group",
    },
)]
/// Get "notes" for a specific backup
pub fn get_notes(
    store: String,
    ns: Option<BackupNamespace>,
    backup_dir: pbs_api_types::BackupDir,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<String, Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let ns = ns.unwrap_or_default();

    let datastore = check_privs_and_load_store(
        &store,
        &ns,
        &auth_id,
        PRIV_DATASTORE_AUDIT,
        PRIV_DATASTORE_BACKUP,
        Some(Operation::Read),
        &backup_dir.group,
    )?;

    let backup_dir = datastore.backup_dir(ns, backup_dir)?;

    let (manifest, _) = backup_dir.load_manifest()?;

    let notes = manifest.unprotected["notes"].as_str().unwrap_or("");

    Ok(String::from(notes))
}

#[api(
    input: {
        properties: {
            store: { schema: DATASTORE_SCHEMA },
            ns: {
                type: BackupNamespace,
                optional: true,
            },
            backup_dir: {
                type: pbs_api_types::BackupDir,
                flatten: true,
            },
            notes: {
                description: "A multiline text.",
            },
        },
    },
    access: {
        permission: &Permission::Anybody,
        description: "Requires on /datastore/{store}[/{namespace}] either DATASTORE_MODIFY for any \
            or DATASTORE_BACKUP and being the owner of the group",
    },
)]
/// Set "notes" for a specific backup
pub fn set_notes(
    store: String,
    ns: Option<BackupNamespace>,
    backup_dir: pbs_api_types::BackupDir,
    notes: String,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let ns = ns.unwrap_or_default();

    let datastore = check_privs_and_load_store(
        &store,
        &ns,
        &auth_id,
        PRIV_DATASTORE_MODIFY,
        PRIV_DATASTORE_BACKUP,
        Some(Operation::Write),
        &backup_dir.group,
    )?;

    let backup_dir = datastore.backup_dir(ns, backup_dir)?;

    backup_dir
        .update_manifest(|manifest| {
            manifest.unprotected["notes"] = notes.into();
        })
        .map_err(|err| format_err!("unable to update manifest blob - {}", err))?;

    Ok(())
}

#[api(
    input: {
        properties: {
            store: { schema: DATASTORE_SCHEMA },
            ns: {
                type: BackupNamespace,
                optional: true,
            },
            backup_dir: {
                type: pbs_api_types::BackupDir,
                flatten: true,
            },
        },
    },
    access: {
        permission: &Permission::Anybody,
        description: "Requires on /datastore/{store}[/{namespace}] either DATASTORE_AUDIT for any \
            or DATASTORE_BACKUP and being the owner of the group",
    },
)]
/// Query protection for a specific backup
pub fn get_protection(
    store: String,
    ns: Option<BackupNamespace>,
    backup_dir: pbs_api_types::BackupDir,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<bool, Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let ns = ns.unwrap_or_default();
    let datastore = check_privs_and_load_store(
        &store,
        &ns,
        &auth_id,
        PRIV_DATASTORE_AUDIT,
        PRIV_DATASTORE_BACKUP,
        Some(Operation::Read),
        &backup_dir.group,
    )?;

    let backup_dir = datastore.backup_dir(ns, backup_dir)?;

    Ok(backup_dir.is_protected())
}

#[api(
    input: {
        properties: {
            store: { schema: DATASTORE_SCHEMA },
            ns: {
                type: BackupNamespace,
                optional: true,
            },
            backup_dir: {
                type: pbs_api_types::BackupDir,
                flatten: true,
            },
            protected: {
                description: "Enable/disable protection.",
            },
        },
    },
    access: {
        permission: &Permission::Anybody,
        description: "Requires on /datastore/{store}[/{namespace}] either DATASTORE_MODIFY for any \
            or DATASTORE_BACKUP and being the owner of the group",
    },
)]
/// En- or disable protection for a specific backup
pub async fn set_protection(
    store: String,
    ns: Option<BackupNamespace>,
    backup_dir: pbs_api_types::BackupDir,
    protected: bool,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;

    tokio::task::spawn_blocking(move || {
        let ns = ns.unwrap_or_default();
        let datastore = check_privs_and_load_store(
            &store,
            &ns,
            &auth_id,
            PRIV_DATASTORE_MODIFY,
            PRIV_DATASTORE_BACKUP,
            Some(Operation::Write),
            &backup_dir.group,
        )?;

        let backup_dir = datastore.backup_dir(ns, backup_dir)?;

        datastore.update_protection(&backup_dir, protected)
    })
    .await?
}

#[api(
    input: {
        properties: {
            store: { schema: DATASTORE_SCHEMA },
            ns: {
                type: BackupNamespace,
                optional: true,
            },
            backup_group: {
                type: pbs_api_types::BackupGroup,
                flatten: true,
            },
            "new-owner": {
                type: Authid,
            },
        },
    },
    access: {
        permission: &Permission::Anybody,
        description: "Datastore.Modify on whole datastore, or changing ownership between user and \
            a user's token for owned backups with Datastore.Backup"
    },
)]
/// Change owner of a backup group
pub async fn set_backup_owner(
    store: String,
    ns: Option<BackupNamespace>,
    backup_group: pbs_api_types::BackupGroup,
    new_owner: Authid,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;

    tokio::task::spawn_blocking(move || {
        let ns = ns.unwrap_or_default();
        let owner_check_required = check_ns_privs_full(
            &store,
            &ns,
            &auth_id,
            PRIV_DATASTORE_MODIFY,
            PRIV_DATASTORE_BACKUP,
        )?;

        let datastore = DataStore::lookup_datastore(&store, Some(Operation::Write))?;

        let backup_group = datastore.backup_group(ns, backup_group);

        if owner_check_required {
            let owner = backup_group.get_owner()?;

            let allowed = match (owner.is_token(), new_owner.is_token()) {
                (true, true) => {
                    // API token to API token, owned by same user
                    let owner = owner.user();
                    let new_owner = new_owner.user();
                    owner == new_owner && Authid::from(owner.clone()) == auth_id
                }
                (true, false) => {
                    // API token to API token owner
                    Authid::from(owner.user().clone()) == auth_id && new_owner == auth_id
                }
                (false, true) => {
                    // API token owner to API token
                    owner == auth_id && Authid::from(new_owner.user().clone()) == auth_id
                }
                (false, false) => {
                    // User to User, not allowed for unprivileged users
                    false
                }
            };

            if !allowed {
                return Err(http_err!(
                    UNAUTHORIZED,
                    "{} does not have permission to change owner of backup group '{}' to {}",
                    auth_id,
                    backup_group.group(),
                    new_owner,
                ));
            }
        }

        let user_info = CachedUserInfo::new()?;

        if !user_info.is_active_auth_id(&new_owner) {
            bail!(
                "{} '{}' is inactive or non-existent",
                if new_owner.is_token() {
                    "API token".to_string()
                } else {
                    "user".to_string()
                },
                new_owner
            );
        }

        backup_group.set_owner(&new_owner, true)?;

        Ok(())
    })
    .await?
}

#[sortable]
const DATASTORE_INFO_SUBDIRS: SubdirMap = &[
    (
        "active-operations",
        &Router::new().get(&API_METHOD_GET_ACTIVE_OPERATIONS),
    ),
    ("catalog", &Router::new().get(&API_METHOD_CATALOG)),
    (
        "change-owner",
        &Router::new().post(&API_METHOD_SET_BACKUP_OWNER),
    ),
    (
        "download",
        &Router::new().download(&API_METHOD_DOWNLOAD_FILE),
    ),
    (
        "download-decoded",
        &Router::new().download(&API_METHOD_DOWNLOAD_FILE_DECODED),
    ),
    ("files", &Router::new().get(&API_METHOD_LIST_SNAPSHOT_FILES)),
    (
        "gc",
        &Router::new()
            .get(&API_METHOD_GARBAGE_COLLECTION_STATUS)
            .post(&API_METHOD_START_GARBAGE_COLLECTION),
    ),
    (
        "group-notes",
        &Router::new()
            .get(&API_METHOD_GET_GROUP_NOTES)
            .put(&API_METHOD_SET_GROUP_NOTES),
    ),
    (
        "groups",
        &Router::new()
            .get(&API_METHOD_LIST_GROUPS)
            .delete(&API_METHOD_DELETE_GROUP),
    ),
    (
        "namespace",
        // FIXME: move into datastore:: sub-module?!
        &crate::api2::admin::namespace::ROUTER,
    ),
    (
        "notes",
        &Router::new()
            .get(&API_METHOD_GET_NOTES)
            .put(&API_METHOD_SET_NOTES),
    ),
    (
        "protected",
        &Router::new()
            .get(&API_METHOD_GET_PROTECTION)
            .put(&API_METHOD_SET_PROTECTION),
    ),
    ("prune", &Router::new().post(&API_METHOD_PRUNE)),
    (
        "prune-datastore",
        &Router::new().post(&API_METHOD_PRUNE_DATASTORE),
    ),
    (
        "pxar-file-download",
        &Router::new().download(&API_METHOD_PXAR_FILE_DOWNLOAD),
    ),
    ("rrd", &Router::new().get(&API_METHOD_GET_RRD_STATS)),
    (
        "snapshots",
        &Router::new()
            .get(&API_METHOD_LIST_SNAPSHOTS)
            .delete(&API_METHOD_DELETE_SNAPSHOT),
    ),
    ("status", &Router::new().get(&API_METHOD_STATUS)),
    (
        "upload-backup-log",
        &Router::new().upload(&API_METHOD_UPLOAD_BACKUP_LOG),
    ),
    ("verify", &Router::new().post(&API_METHOD_VERIFY)),
];

const DATASTORE_INFO_ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(DATASTORE_INFO_SUBDIRS))
    .subdirs(DATASTORE_INFO_SUBDIRS);

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_GET_DATASTORE_LIST)
    .match_all("store", &DATASTORE_INFO_ROUTER);
