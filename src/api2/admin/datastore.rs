use std::collections::{HashSet, HashMap};
use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::sync::{Arc, Mutex};
use std::path::{Path, PathBuf};
use std::pin::Pin;

use anyhow::{bail, format_err, Error};
use futures::*;
use hyper::http::request::Parts;
use hyper::{header, Body, Response, StatusCode};
use serde_json::{json, Value};

use proxmox::api::{
    api, ApiResponseFuture, ApiHandler, ApiMethod, Router,
    RpcEnvironment, RpcEnvironmentType, Permission
};
use proxmox::api::router::SubdirMap;
use proxmox::api::schema::*;
use proxmox::tools::fs::{replace_file, CreateOptions};
use proxmox::{http_err, identity, list_subdirs_api_method, sortable};

use pxar::accessor::aio::{Accessor, FileContents, FileEntry};
use pxar::EntryKind;

use crate::api2::types::*;
use crate::api2::node::rrd::create_value_from_rrd;
use crate::backup::*;
use crate::config::datastore;
use crate::config::cached_user_info::CachedUserInfo;

use crate::server::{jobstate::Job, WorkerTask};
use crate::tools::{
    self,
    zip::{ZipEncoder, ZipEntry},
    AsyncChannelWriter, AsyncReaderStream, WrappedReaderStream,
};

use crate::config::acl::{
    PRIV_DATASTORE_AUDIT,
    PRIV_DATASTORE_MODIFY,
    PRIV_DATASTORE_READ,
    PRIV_DATASTORE_PRUNE,
    PRIV_DATASTORE_BACKUP,
    PRIV_DATASTORE_VERIFY,
};

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

fn check_backup_owner(
    owner: &Authid,
    auth_id: &Authid,
) -> Result<(), Error> {
    let correct_owner = owner == auth_id
        || (owner.is_token() && &Authid::from(owner.user().clone()) == auth_id);
    if !correct_owner {
        bail!("backup owner check failed ({} != {})", auth_id, owner);
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

fn group_backups(backup_list: Vec<BackupInfo>) -> HashMap<String, Vec<BackupInfo>> {

    let mut group_hash = HashMap::new();

    for info in backup_list {
        let group_id = info.backup_dir.group().group_path().to_str().unwrap().to_owned();
        let time_list = group_hash.entry(group_id).or_insert(vec![]);
        time_list.push(info);
    }

    group_hash
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
        type: Array,
        description: "Returns the list of backup groups.",
        items: {
            type: GroupListItem,
        }
    },
    access: {
        permission: &Permission::Privilege(
            &["datastore", "{store}"],
            PRIV_DATASTORE_AUDIT | PRIV_DATASTORE_BACKUP,
            true),
    },
)]
/// List backup groups.
fn list_groups(
    store: String,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<GroupListItem>, Error> {

    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let user_info = CachedUserInfo::new()?;
    let user_privs = user_info.lookup_privs(&auth_id, &["datastore", &store]);

    let datastore = DataStore::lookup_datastore(&store)?;

    let backup_list = BackupInfo::list_backups(&datastore.base_path())?;

    let group_hash = group_backups(backup_list);

    let mut groups = Vec::new();

    for (_group_id, mut list) in group_hash {

        BackupInfo::sort_list(&mut list, false);

        let info = &list[0];

        let group = info.backup_dir.group();

        let list_all = (user_privs & PRIV_DATASTORE_AUDIT) != 0;
        let owner = datastore.get_owner(group)?;
        if !list_all && check_backup_owner(&owner, &auth_id).is_err() {
            continue;
        }

        let result_item = GroupListItem {
            backup_type: group.backup_type().to_string(),
            backup_id: group.backup_id().to_string(),
            last_backup: info.backup_dir.backup_time(),
            backup_count: list.len() as u64,
            files: info.files.clone(),
            owner: Some(owner),
        };
        groups.push(result_item);
    }

    Ok(groups)
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
    returns: {
        type: Array,
        description: "Returns the list of archive files inside a backup snapshots.",
        items: {
            type: BackupContent,
        }
    },
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
fn delete_snapshot(
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
    returns: {
        type: Array,
        description: "Returns the list of snapshots.",
        items: {
            type: SnapshotListItem,
        }
    },
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

    let datastore = DataStore::lookup_datastore(&store)?;

    let base_path = datastore.base_path();

    let backup_list = BackupInfo::list_backups(&base_path)?;

    let mut snapshots = vec![];

    for info in backup_list {
        let group = info.backup_dir.group();
        if let Some(ref backup_type) = backup_type {
            if backup_type != group.backup_type() { continue; }
        }
        if let Some(ref backup_id) = backup_id {
            if backup_id != group.backup_id() { continue; }
        }

        let list_all = (user_privs & PRIV_DATASTORE_AUDIT) != 0;
        let owner = datastore.get_owner(group)?;

        if !list_all && check_backup_owner(&owner, &auth_id).is_err() {
            continue;
        }

        let mut size = None;

        let (comment, verification, files) = match get_all_snapshot_files(&datastore, &info) {
            Ok((manifest, files)) => {
                size = Some(files.iter().map(|x| x.size.unwrap_or(0)).sum());
                // extract the first line from notes
                let comment: Option<String> = manifest.unprotected["notes"]
                    .as_str()
                    .and_then(|notes| notes.lines().next())
                    .map(String::from);

                let verify = manifest.unprotected["verify_state"].clone();
                let verify: Option<SnapshotVerifyState> = match serde_json::from_value(verify) {
                    Ok(verify) => verify,
                    Err(err) => {
                        eprintln!("error parsing verification state : '{}'", err);
                        None
                    }
                };

                (comment, verify, files)
            },
            Err(err) => {
                eprintln!("error during snapshot file listing: '{}'", err);
                (
                    None,
                    None,
                    info
                        .files
                        .iter()
                        .map(|x| BackupContent {
                            filename: x.to_string(),
                            size: None,
                            crypt_mode: None,
                        })
                        .collect()
                )
            },
        };

        let result_item = SnapshotListItem {
            backup_type: group.backup_type().to_string(),
            backup_id: group.backup_id().to_string(),
            backup_time: info.backup_dir.backup_time(),
            comment,
            verification,
            files,
            size,
            owner: Some(owner),
        };

        snapshots.push(result_item);
    }

    Ok(snapshots)
}

fn get_snapshots_count(store: &DataStore) -> Result<Counts, Error> {
    let base_path = store.base_path();
    let backup_list = BackupInfo::list_backups(&base_path)?;
    let mut groups = HashSet::new();

    let mut result = Counts {
        ct: None,
        host: None,
        vm: None,
        other: None,
    };

    for info in backup_list {
        let group = info.backup_dir.group();

        let id = group.backup_id();
        let backup_type = group.backup_type();

        let mut new_id = false;

        if groups.insert(format!("{}-{}", &backup_type, &id)) {
            new_id = true;
        }

        let mut counts = match backup_type {
            "ct" => result.ct.take().unwrap_or(Default::default()),
            "host" => result.host.take().unwrap_or(Default::default()),
            "vm" => result.vm.take().unwrap_or(Default::default()),
            _ => result.other.take().unwrap_or(Default::default()),
        };

        counts.snapshots += 1;
        if new_id {
            counts.groups +=1;
        }

        match backup_type {
            "ct" => result.ct = Some(counts),
            "host" => result.host = Some(counts),
            "vm" => result.vm = Some(counts),
            _ => result.other = Some(counts),
        }
    }

    Ok(result)
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
        type: DataStoreStatus,
    },
    access: {
        permission: &Permission::Privilege(&["datastore", "{store}"], PRIV_DATASTORE_AUDIT | PRIV_DATASTORE_BACKUP, true),
    },
)]
/// Get datastore status.
pub fn status(
    store: String,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<DataStoreStatus, Error> {
    let datastore = DataStore::lookup_datastore(&store)?;
    let storage = crate::tools::disks::disk_usage(&datastore.base_path())?;
    let counts = get_snapshots_count(&datastore)?;
    let gc_status = datastore.last_gc_status();

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
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {
    let datastore = DataStore::lookup_datastore(&store)?;

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

    let to_stdout = if rpcenv.env_type() == RpcEnvironmentType::CLI { true } else { false };

    let upid_str = WorkerTask::new_thread(
        worker_type,
        Some(worker_id.clone()),
        auth_id.clone(),
        to_stdout,
        move |worker| {
            let verified_chunks = Arc::new(Mutex::new(HashSet::with_capacity(1024*16)));
            let corrupt_chunks = Arc::new(Mutex::new(HashSet::with_capacity(64)));

            let failed_dirs = if let Some(backup_dir) = backup_dir {
                let mut res = Vec::new();
                if !verify_backup_dir(
                    datastore,
                    &backup_dir,
                    verified_chunks,
                    corrupt_chunks,
                    worker.clone(),
                    worker.upid().clone(),
                    None,
                )? {
                    res.push(backup_dir.to_string());
                }
                res
            } else if let Some(backup_group) = backup_group {
                let (_count, failed_dirs) = verify_backup_group(
                    datastore,
                    &backup_group,
                    verified_chunks,
                    corrupt_chunks,
                    None,
                    worker.clone(),
                    worker.upid(),
                    None,
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

                verify_all_backups(datastore, worker.clone(), worker.upid(), owner, None)?
            };
            if failed_dirs.len() > 0 {
                worker.log("Failed to verify following snapshots:");
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

#[macro_export]
macro_rules! add_common_prune_prameters {
    ( [ $( $list1:tt )* ] ) => {
        add_common_prune_prameters!([$( $list1 )* ] ,  [])
    };
    ( [ $( $list1:tt )* ] ,  [ $( $list2:tt )* ] ) => {
        [
            $( $list1 )*
            (
                "keep-daily",
                true,
                &PRUNE_SCHEMA_KEEP_DAILY,
            ),
            (
                "keep-hourly",
                true,
                &PRUNE_SCHEMA_KEEP_HOURLY,
            ),
            (
                "keep-last",
                true,
                &PRUNE_SCHEMA_KEEP_LAST,
            ),
            (
                "keep-monthly",
                true,
                &PRUNE_SCHEMA_KEEP_MONTHLY,
            ),
            (
                "keep-weekly",
                true,
                &PRUNE_SCHEMA_KEEP_WEEKLY,
            ),
            (
                "keep-yearly",
                true,
                &PRUNE_SCHEMA_KEEP_YEARLY,
            ),
            $( $list2 )*
        ]
    }
}

pub const API_RETURN_SCHEMA_PRUNE: Schema = ArraySchema::new(
    "Returns the list of snapshots and a flag indicating if there are kept or removed.",
    &PruneListItem::API_SCHEMA
).schema();

const API_METHOD_PRUNE: ApiMethod = ApiMethod::new(
    &ApiHandler::Sync(&prune),
    &ObjectSchema::new(
        "Prune the datastore.",
        &add_common_prune_prameters!([
            ("backup-id", false, &BACKUP_ID_SCHEMA),
            ("backup-type", false, &BACKUP_TYPE_SCHEMA),
            ("dry-run", true, &BooleanSchema::new(
                "Just show what prune would do, but do not delete anything.")
             .schema()
            ),
        ],[
            ("store", false, &DATASTORE_SCHEMA),
        ])
    ))
    .returns(&API_RETURN_SCHEMA_PRUNE)
    .access(None, &Permission::Privilege(
    &["datastore", "{store}"],
    PRIV_DATASTORE_MODIFY | PRIV_DATASTORE_PRUNE,
    true)
);

fn prune(
    param: Value,
    _info: &ApiMethod,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let store = tools::required_string_param(&param, "store")?;
    let backup_type = tools::required_string_param(&param, "backup-type")?;
    let backup_id = tools::required_string_param(&param, "backup-id")?;

    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;

    let dry_run = param["dry-run"].as_bool().unwrap_or(false);

    let group = BackupGroup::new(backup_type, backup_id);

    let datastore = DataStore::lookup_datastore(&store)?;

    check_priv_or_backup_owner(&datastore, &group, &auth_id, PRIV_DATASTORE_MODIFY)?;

    let prune_options = PruneOptions {
        keep_last: param["keep-last"].as_u64(),
        keep_hourly: param["keep-hourly"].as_u64(),
        keep_daily: param["keep-daily"].as_u64(),
        keep_weekly: param["keep-weekly"].as_u64(),
        keep_monthly: param["keep-monthly"].as_u64(),
        keep_yearly: param["keep-yearly"].as_u64(),
    };

    let worker_id = format!("{}:{}/{}", store, backup_type, backup_id);

    let mut prune_result = Vec::new();

    let list = group.list_backups(&datastore.base_path())?;

    let mut prune_info = compute_prune_info(list, &prune_options)?;

    prune_info.reverse(); // delete older snapshots first

    let keep_all = !prune_options.keeps_something();

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
    let worker = WorkerTask::new("prune", Some(worker_id), auth_id.clone(), true)?;

    if keep_all {
        worker.log("No prune selection - keeping all files.");
    } else {
        worker.log(format!("retention options: {}", prune_options.cli_options_string()));
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
fn start_garbage_collection(
    store: String,
    _info: &ApiMethod,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let datastore = DataStore::lookup_datastore(&store)?;
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;

    let job =  Job::new("garbage_collection", &store)
        .map_err(|_| format_err!("garbage collection already running"))?;

    let to_stdout = if rpcenv.env_type() == RpcEnvironmentType::CLI { true } else { false };

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
        items: {
            description: "Datastore name and description.",
            properties: {
                store: {
                    schema: DATASTORE_SCHEMA,
                },
                comment: {
                    optional: true,
                    schema: SINGLE_LINE_COMMENT_SCHEMA,
                },
            },
        },
    },
    access: {
        permission: &Permission::Anybody,
    },
)]
/// Datastore list
fn get_datastore_list(
    _param: Value,
    _info: &ApiMethod,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let (config, _digest) = datastore::config()?;

    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let user_info = CachedUserInfo::new()?;

    let mut list = Vec::new();

    for (store, (_, data)) in &config.sections {
        let user_privs = user_info.lookup_privs(&auth_id, &["datastore", &store]);
        let allowed = (user_privs & (PRIV_DATASTORE_AUDIT| PRIV_DATASTORE_BACKUP)) != 0;
        if allowed {
            let mut entry = json!({ "store": store });
            if let Some(comment) = data["comment"].as_str() {
                entry["comment"] = comment.into();
            }
            list.push(entry);
        }
    }

    Ok(list.into())
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

fn download_file(
    _parts: Parts,
    _req_body: Body,
    param: Value,
    _info: &ApiMethod,
    rpcenv: Box<dyn RpcEnvironment>,
) -> ApiResponseFuture {

    async move {
        let store = tools::required_string_param(&param, "store")?;
        let datastore = DataStore::lookup_datastore(store)?;

        let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;

        let file_name = tools::required_string_param(&param, "file-name")?.to_owned();

        let backup_type = tools::required_string_param(&param, "backup-type")?;
        let backup_id = tools::required_string_param(&param, "backup-id")?;
        let backup_time = tools::required_integer_param(&param, "backup-time")?;

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
            .map_ok(|bytes| hyper::body::Bytes::from(bytes.freeze()))
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

fn download_file_decoded(
    _parts: Parts,
    _req_body: Body,
    param: Value,
    _info: &ApiMethod,
    rpcenv: Box<dyn RpcEnvironment>,
) -> ApiResponseFuture {

    async move {
        let store = tools::required_string_param(&param, "store")?;
        let datastore = DataStore::lookup_datastore(store)?;

        let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;

        let file_name = tools::required_string_param(&param, "file-name")?.to_owned();

        let backup_type = tools::required_string_param(&param, "backup-type")?;
        let backup_id = tools::required_string_param(&param, "backup-id")?;
        let backup_time = tools::required_integer_param(&param, "backup-time")?;

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
                let reader = AsyncIndexReader::new(index, chunk_reader);
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
                let reader = AsyncIndexReader::new(index, chunk_reader);
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

fn upload_backup_log(
    _parts: Parts,
    req_body: Body,
    param: Value,
    _info: &ApiMethod,
    rpcenv: Box<dyn RpcEnvironment>,
) -> ApiResponseFuture {

    async move {
        let store = tools::required_string_param(&param, "store")?;
        let datastore = DataStore::lookup_datastore(store)?;

        let file_name =  CLIENT_LOG_BLOB_NAME;

        let backup_type = tools::required_string_param(&param, "backup-type")?;
        let backup_id = tools::required_string_param(&param, "backup-id")?;
        let backup_time = tools::required_integer_param(&param, "backup-time")?;

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
        Ok(crate::server::formatter::json_response(Ok(Value::Null)))
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
fn catalog(
    store: String,
    backup_type: String,
    backup_id: String,
    backup_time: i64,
    filepath: String,
    _param: Value,
    _info: &ApiMethod,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {
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
    let mut current = catalog_reader.root()?;
    let mut components = vec![];


    if filepath != "root" {
        components = base64::decode(filepath)?;
        if components.len() > 0 && components[0] == '/' as u8 {
            components.remove(0);
        }
        for component in components.split(|c| *c == '/' as u8) {
            if let Some(entry) = catalog_reader.lookup(&current, component)? {
                current = entry;
            } else {
                bail!("path {:?} not found in catalog", &String::from_utf8_lossy(&components));
            }
        }
    }

    let mut res = Vec::new();

    for direntry in catalog_reader.read_dir(&current)? {
        let mut components = components.clone();
        components.push('/' as u8);
        components.extend(&direntry.name);
        let path = base64::encode(components);
        let text = String::from_utf8_lossy(&direntry.name);
        let mut entry = json!({
            "filepath": path,
            "text": text,
            "type": CatalogEntryType::from(&direntry.attr).to_string(),
            "leaf": true,
        });
        match direntry.attr {
            DirEntryAttribute::Directory { start: _ } => {
                entry["leaf"] = false.into();
            },
            DirEntryAttribute::File { size, mtime } => {
                entry["size"] = size.into();
                entry["mtime"] = mtime.into();
            },
            _ => {},
        }
        res.push(entry);
    }

    Ok(res.into())
}

fn recurse_files<'a, T, W>(
    zip: &'a mut ZipEncoder<W>,
    decoder: &'a mut Accessor<T>,
    prefix: &'a Path,
    file: FileEntry<T>,
) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send + 'a>>
where
    T: Clone + pxar::accessor::ReadAt + Unpin + Send + Sync + 'static,
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    Box::pin(async move {
        let metadata = file.entry().metadata();
        let path = file.entry().path().strip_prefix(&prefix)?.to_path_buf();

        match file.kind() {
            EntryKind::File { .. } => {
                let entry = ZipEntry::new(
                    path,
                    metadata.stat.mtime.secs,
                    metadata.stat.mode as u16,
                    true,
                );
                zip.add_entry(entry, Some(file.contents().await?))
                   .await
                   .map_err(|err| format_err!("could not send file entry: {}", err))?;
            }
            EntryKind::Hardlink(_) => {
                let realfile = decoder.follow_hardlink(&file).await?;
                let entry = ZipEntry::new(
                    path,
                    metadata.stat.mtime.secs,
                    metadata.stat.mode as u16,
                    true,
                );
                zip.add_entry(entry, Some(realfile.contents().await?))
                   .await
                   .map_err(|err| format_err!("could not send file entry: {}", err))?;
            }
            EntryKind::Directory => {
                let dir = file.enter_directory().await?;
                let mut readdir = dir.read_dir();
                let entry = ZipEntry::new(
                    path,
                    metadata.stat.mtime.secs,
                    metadata.stat.mode as u16,
                    false,
                );
                zip.add_entry::<FileContents<T>>(entry, None).await?;
                while let Some(entry) = readdir.next().await {
                    let entry = entry?.decode_entry().await?;
                    recurse_files(zip, decoder, prefix, entry).await?;
                }
            }
            _ => {} // ignore all else
        };

        Ok(())
    })
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

fn pxar_file_download(
    _parts: Parts,
    _req_body: Body,
    param: Value,
    _info: &ApiMethod,
    rpcenv: Box<dyn RpcEnvironment>,
) -> ApiResponseFuture {

    async move {
        let store = tools::required_string_param(&param, "store")?;
        let datastore = DataStore::lookup_datastore(&store)?;

        let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;

        let filepath = tools::required_string_param(&param, "filepath")?.to_owned();

        let backup_type = tools::required_string_param(&param, "backup-type")?;
        let backup_id = tools::required_string_param(&param, "backup-id")?;
        let backup_time = tools::required_integer_param(&param, "backup-time")?;

        let backup_dir = BackupDir::new(backup_type, backup_id, backup_time)?;

        check_priv_or_backup_owner(&datastore, backup_dir.group(), &auth_id, PRIV_DATASTORE_READ)?;

        let mut components = base64::decode(&filepath)?;
        if components.len() > 0 && components[0] == '/' as u8 {
            components.remove(0);
        }

        let mut split = components.splitn(2, |c| *c == '/' as u8);
        let pxar_name = std::str::from_utf8(split.next().unwrap())?;
        let file_path = split.next().ok_or(format_err!("filepath looks strange '{}'", filepath))?;
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
        let file = root
            .lookup(OsStr::from_bytes(file_path)).await?
            .ok_or(format_err!("error opening '{:?}'", file_path))?;

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
                            filepath, err
                        );
                        err
                    }),
            ),
            EntryKind::Directory => {
                let (sender, receiver) = tokio::sync::mpsc::channel(100);
                let mut prefix = PathBuf::new();
                let mut components = file.entry().path().components();
                components.next_back(); // discar last
                for comp in components {
                    prefix.push(comp);
                }

                let channelwriter = AsyncChannelWriter::new(sender, 1024 * 1024);

                crate::server::spawn_internal_task(async move {
                    let mut zipencoder = ZipEncoder::new(channelwriter);
                    let mut decoder = decoder;
                    recurse_files(&mut zipencoder, &mut decoder, &prefix, file)
                        .await
                        .map_err(|err| eprintln!("error during creating of zip: {}", err))?;

                    zipencoder
                        .finish()
                        .await
                        .map_err(|err| eprintln!("error during finishing of zip: {}", err))
                });

                Body::wrap_stream(receiver.map_err(move |err| {
                    eprintln!("error during streaming of zip '{:?}' - {}", filepath, err);
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
fn get_rrd_stats(
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
fn get_notes(
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
fn set_notes(
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
fn set_backup_owner(
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
        "groups",
        &Router::new()
            .get(&API_METHOD_LIST_GROUPS)
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
