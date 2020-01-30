use std::collections::{HashSet, HashMap};
use std::convert::TryFrom;

use chrono::{TimeZone, Local};
use failure::*;
use futures::*;
use hyper::http::request::Parts;
use hyper::{header, Body, Response, StatusCode};
use serde_json::{json, Value};

use proxmox::api::api;
use proxmox::api::{ApiResponseFuture, ApiHandler, ApiMethod, Router, RpcEnvironment, RpcEnvironmentType};
use proxmox::api::router::SubdirMap;
use proxmox::api::schema::*;
use proxmox::tools::fs::{file_get_contents, replace_file, CreateOptions};
use proxmox::try_block;
use proxmox::{http_err, identity, list_subdirs_api_method, sortable};

use crate::api2::types::*;
use crate::backup::*;
use crate::config::datastore;
use crate::server::WorkerTask;
use crate::tools;

fn read_backup_index(store: &DataStore, backup_dir: &BackupDir) -> Result<Vec<BackupContent>, Error> {

    let mut path = store.base_path();
    path.push(backup_dir.relative_path());
    path.push("index.json.blob");

    let raw_data = file_get_contents(&path)?;
    let index_size = raw_data.len() as u64;
    let blob = DataBlob::from_raw(raw_data)?;

    let manifest = BackupManifest::try_from(blob)?;

    let mut result = Vec::new();
    for item in manifest.files() {
        result.push(BackupContent {
            filename: item.filename.clone(),
            size: Some(item.size),
        });
    }

    result.push(BackupContent {
        filename: "index.json.blob".to_string(),
        size: Some(index_size),
    });

    Ok(result)
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
)]
/// List backup groups.
fn list_groups(
    store: String,
) -> Result<Vec<GroupListItem>, Error> {

    let datastore = DataStore::lookup_datastore(&store)?;

    let backup_list = BackupInfo::list_backups(&datastore.base_path())?;

    let group_hash = group_backups(backup_list);

    let mut groups = Vec::new();

    for (_group_id, mut list) in group_hash {

        BackupInfo::sort_list(&mut list, false);

        let info = &list[0];
        let group = info.backup_dir.group();

        let result_item = GroupListItem {
            backup_type: group.backup_type().to_string(),
            backup_id: group.backup_id().to_string(),
            last_backup: info.backup_dir.backup_time().timestamp(),
            backup_count: list.len() as u64,
            files: info.files.clone(),
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
)]
/// List snapshot files.
fn list_snapshot_files(
    store: String,
    backup_type: String,
    backup_id: String,
    backup_time: i64,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<BackupContent>, Error> {

    let datastore = DataStore::lookup_datastore(&store)?;
    let snapshot = BackupDir::new(backup_type, backup_id, backup_time);

    let mut files = read_backup_index(&datastore, &snapshot)?;

    let info = BackupInfo::new(&datastore.base_path(), snapshot)?;

    let file_set = files.iter().fold(HashSet::new(), |mut acc, item| {
        acc.insert(item.filename.clone());
        acc
    });

    for file in info.files {
        if file_set.contains(&file) { continue; }
        files.push(BackupContent { filename: file, size: None });
    }

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
)]
/// Delete backup snapshot.
fn delete_snapshot(
    store: String,
    backup_type: String,
    backup_id: String,
    backup_time: i64,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let snapshot = BackupDir::new(backup_type, backup_id, backup_time);

    let datastore = DataStore::lookup_datastore(&store)?;

    datastore.remove_backup_dir(&snapshot)?;

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
)]
/// List backup snapshots.
fn list_snapshots (
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<SnapshotListItem>, Error> {

    let store = tools::required_string_param(&param, "store")?;
    let backup_type = param["backup-type"].as_str();
    let backup_id = param["backup-id"].as_str();

    let datastore = DataStore::lookup_datastore(store)?;

    let base_path = datastore.base_path();

    let backup_list = BackupInfo::list_backups(&base_path)?;

    let mut snapshots = vec![];

    for info in backup_list {
        let group = info.backup_dir.group();
        if let Some(backup_type) = backup_type {
            if backup_type != group.backup_type() { continue; }
        }
        if let Some(backup_id) = backup_id {
            if backup_id != group.backup_id() { continue; }
        }

        let mut result_item = SnapshotListItem {
            backup_type: group.backup_type().to_string(),
            backup_id: group.backup_id().to_string(),
            backup_time: info.backup_dir.backup_time().timestamp(),
            files: info.files,
            size: None,
        };

        if let Ok(index) = read_backup_index(&datastore, &info.backup_dir) {
            let mut backup_size = 0;
            for item in index.iter() {
                if let Some(item_size) = item.size {
                    backup_size += item_size;
                }
            }
            result_item.size = Some(backup_size);
        }

        snapshots.push(result_item);
    }

    Ok(snapshots)
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
        type: StorageStatus,
    },
)]
/// Get datastore status.
fn status(
    store: String,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<StorageStatus, Error> {

    let datastore = DataStore::lookup_datastore(&store)?;

    let base_path = datastore.base_path();

    let mut stat: libc::statfs64 = unsafe { std::mem::zeroed() };

    use nix::NixPath;

    let res = base_path.with_nix_path(|cstr| unsafe { libc::statfs64(cstr.as_ptr(), &mut stat) })?;
    nix::errno::Errno::result(res)?;

    let bsize = stat.f_bsize as u64;

    Ok(StorageStatus {
        total: stat.f_blocks*bsize,
        used: (stat.f_blocks-stat.f_bfree)*bsize,
        avail: stat.f_bavail*bsize,
    })
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
                &IntegerSchema::new("Number of daily backups to keep.")
                    .minimum(1)
                    .schema()
            ),
            (
                "keep-hourly",
                true,
                &IntegerSchema::new("Number of hourly backups to keep.")
                    .minimum(1)
                    .schema()
            ),
            (
                "keep-last",
                true,
                &IntegerSchema::new("Number of backups to keep.")
                    .minimum(1)
                    .schema()
            ),
            (
                "keep-monthly",
                true,
                &IntegerSchema::new("Number of monthly backups to keep.")
                    .minimum(1)
                    .schema()
            ),
            (
                "keep-weekly",
                true,
                &IntegerSchema::new("Number of weekly backups to keep.")
                    .minimum(1)
                    .schema()
            ),
            (
                "keep-yearly",
                true,
                &IntegerSchema::new("Number of yearly backups to keep.")
                    .minimum(1)
                    .schema()
            ),
            $( $list2 )*
        ]
    }
}

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
    )
);

fn prune(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let store = param["store"].as_str().unwrap();

    let backup_type = tools::required_string_param(&param, "backup-type")?;
    let backup_id = tools::required_string_param(&param, "backup-id")?;

    let dry_run = param["dry-run"].as_bool().unwrap_or(false);

    let group = BackupGroup::new(backup_type, backup_id);

    let datastore = DataStore::lookup_datastore(store)?;

    let prune_options = PruneOptions {
        keep_last: param["keep-last"].as_u64(),
        keep_hourly: param["keep-hourly"].as_u64(),
        keep_daily: param["keep-daily"].as_u64(),
        keep_weekly: param["keep-weekly"].as_u64(),
        keep_monthly: param["keep-monthly"].as_u64(),
        keep_yearly: param["keep-yearly"].as_u64(),
    };

    let worker_id = format!("{}_{}_{}", store, backup_type, backup_id);

    // We use a WorkerTask just to have a task log, but run synchrounously
    let worker = WorkerTask::new("prune", Some(worker_id), "root@pam", true)?;
    let result = try_block! {
        if !prune_options.keeps_something() {
            worker.log("No prune selection - keeping all files.");
            return Ok(());
        } else {
            worker.log(format!("retention options: {}", prune_options.cli_options_string()));
            if dry_run {
                worker.log(format!("Testing prune on store \"{}\" group \"{}/{}\"",
                                   store, backup_type, backup_id));
            } else {
                worker.log(format!("Starting prune on store \"{}\" group \"{}/{}\"",
                                   store, backup_type, backup_id));
            }
        }

        let list = group.list_backups(&datastore.base_path())?;

        let mut prune_info = compute_prune_info(list, &prune_options)?;

        prune_info.reverse(); // delete older snapshots first

        for (info, keep) in prune_info {
            let backup_time = info.backup_dir.backup_time();
            let timestamp = BackupDir::backup_time_to_string(backup_time);
            let group = info.backup_dir.group();

            let msg = format!(
                "{}/{}/{} {}",
                group.backup_type(),
                group.backup_id(),
                timestamp,
                if keep { "keep" } else { "remove" },
            );

            worker.log(msg);

            if !(dry_run || keep) {
                datastore.remove_backup_dir(&info.backup_dir)?;
            }
        }

        Ok(())
    };

    worker.log_result(&result);

    if let Err(err) = result {
        bail!("prune failed - {}", err);
    }

    Ok(json!(worker.to_string())) // return the UPID
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
)]
/// Start garbage collection.
fn start_garbage_collection(
    store: String,
    _info: &ApiMethod,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let datastore = DataStore::lookup_datastore(&store)?;

    println!("Starting garbage collection on store {}", store);

    let to_stdout = if rpcenv.env_type() == RpcEnvironmentType::CLI { true } else { false };

    let upid_str = WorkerTask::new_thread(
        "garbage_collection", Some(store.clone()), "root@pam", to_stdout, move |worker|
        {
            worker.log(format!("starting garbage collection on store {}", store));
            datastore.garbage_collection(worker)
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
    }
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


fn get_datastore_list(
    _param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let (config, _digest) = datastore::config()?;

    Ok(config.convert_to_array("store", None, &[]))
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
);

fn download_file(
    _parts: Parts,
    _req_body: Body,
    param: Value,
    _info: &ApiMethod,
    _rpcenv: Box<dyn RpcEnvironment>,
) -> ApiResponseFuture {

    async move {
        let store = tools::required_string_param(&param, "store")?;

        let datastore = DataStore::lookup_datastore(store)?;

        let file_name = tools::required_string_param(&param, "file-name")?.to_owned();

        let backup_type = tools::required_string_param(&param, "backup-type")?;
        let backup_id = tools::required_string_param(&param, "backup-id")?;
        let backup_time = tools::required_integer_param(&param, "backup-time")?;

        println!("Download {} from {} ({}/{}/{}/{})", file_name, store,
                 backup_type, backup_id, Local.timestamp(backup_time, 0), file_name);

        let backup_dir = BackupDir::new(backup_type, backup_id, backup_time);

        let mut path = datastore.base_path();
        path.push(backup_dir.relative_path());
        path.push(&file_name);

        let file = tokio::fs::File::open(path)
            .map_err(|err| http_err!(BAD_REQUEST, format!("File open failed: {}", err)))
            .await?;

        let payload = tokio_util::codec::FramedRead::new(file, tokio_util::codec::BytesCodec::new())
            .map_ok(|bytes| hyper::body::Bytes::from(bytes.freeze()));
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
pub const API_METHOD_UPLOAD_BACKUP_LOG: ApiMethod = ApiMethod::new(
    &ApiHandler::AsyncHttp(&upload_backup_log),
    &ObjectSchema::new(
        "Download single raw file from backup snapshot.",
        &sorted!([
            ("store", false, &DATASTORE_SCHEMA),
            ("backup-type", false, &BACKUP_TYPE_SCHEMA),
            ("backup-id", false, &BACKUP_ID_SCHEMA),
            ("backup-time", false, &BACKUP_TIME_SCHEMA),
        ]),
    )
);

fn upload_backup_log(
    _parts: Parts,
    req_body: Body,
    param: Value,
    _info: &ApiMethod,
    _rpcenv: Box<dyn RpcEnvironment>,
) -> ApiResponseFuture {

    async move {
        let store = tools::required_string_param(&param, "store")?;

        let datastore = DataStore::lookup_datastore(store)?;

        let file_name = "client.log.blob";

        let backup_type = tools::required_string_param(&param, "backup-type")?;
        let backup_id = tools::required_string_param(&param, "backup-id")?;
        let backup_time = tools::required_integer_param(&param, "backup-time")?;

        let backup_dir = BackupDir::new(backup_type, backup_id, backup_time);

        let mut path = datastore.base_path();
        path.push(backup_dir.relative_path());
        path.push(&file_name);

        if path.exists() {
            bail!("backup already contains a log.");
        }

        println!("Upload backup log to {}/{}/{}/{}/{}", store,
                 backup_type, backup_id, BackupDir::backup_time_to_string(backup_dir.backup_time()), file_name);

        let data = req_body
            .map_err(Error::from)
            .try_fold(Vec::new(), |mut acc, chunk| {
                acc.extend_from_slice(&*chunk);
                future::ok::<_, Error>(acc)
            })
            .await?;

        let blob = DataBlob::from_raw(data)?;
        // always verify CRC at server side
        blob.verify_crc()?;
        let raw_data = blob.raw_data();
        replace_file(&path, raw_data, CreateOptions::new())?;

        // fixme: use correct formatter
        Ok(crate::server::formatter::json_response(Ok(Value::Null)))
    }.boxed()
}

#[sortable]
const DATASTORE_INFO_SUBDIRS: SubdirMap = &[
    (
        "download",
        &Router::new()
            .download(&API_METHOD_DOWNLOAD_FILE)
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
        "prune",
        &Router::new()
            .post(&API_METHOD_PRUNE)
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
];

const DATASTORE_INFO_ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(DATASTORE_INFO_SUBDIRS))
    .subdirs(DATASTORE_INFO_SUBDIRS);


pub const ROUTER: Router = Router::new()
    .get(
        &ApiMethod::new(
            &ApiHandler::Sync(&get_datastore_list),
            &ObjectSchema::new("Directory index.", &[])
        )
    )
    .match_all("store", &DATASTORE_INFO_ROUTER);
