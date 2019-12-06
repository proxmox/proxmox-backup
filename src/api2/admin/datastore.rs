use std::collections::{HashSet, HashMap};

use chrono::{TimeZone, Local};
use failure::*;
use futures::*;
use hyper::http::request::Parts;
use hyper::{header, Body, Response, StatusCode};
use serde_json::{json, Value};

use proxmox::{sortable, identity};
use proxmox::api::{http_err, list_subdirs_api_method};
use proxmox::api::{ApiFuture, ApiHandler, ApiMethod, Router, RpcEnvironment, RpcEnvironmentType};
use proxmox::api::router::SubdirMap;
use proxmox::api::schema::*;
use proxmox::tools::{try_block, fs::file_get_contents, fs::file_set_contents};

use crate::api2::types::*;
use crate::backup::*;
use crate::config::datastore;
use crate::server::WorkerTask;
use crate::tools;

fn read_backup_index(store: &DataStore, backup_dir: &BackupDir) -> Result<Value, Error> {

    let mut path = store.base_path();
    path.push(backup_dir.relative_path());
    path.push("index.json.blob");

    let raw_data = file_get_contents(&path)?;
    let data = DataBlob::from_raw(raw_data)?.decode(None)?;
    let index_size = data.len();
    let mut result: Value = serde_json::from_reader(&mut &data[..])?;

    let mut result = result["files"].take();

    if result == Value::Null {
        bail!("missing 'files' property in backup index {:?}", path);
    }

    result.as_array_mut().unwrap().push(json!({
        "filename": "index.json.blob",
        "size": index_size,
    }));

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

fn list_groups(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let store = param["store"].as_str().unwrap();

    let datastore = DataStore::lookup_datastore(store)?;

    let backup_list = BackupInfo::list_backups(&datastore.base_path())?;

    let group_hash = group_backups(backup_list);

    let mut groups = vec![];

    for (_group_id, mut list) in group_hash {

        BackupInfo::sort_list(&mut list, false);

        let info = &list[0];
        let group = info.backup_dir.group();

        groups.push(json!({
            "backup-type": group.backup_type(),
            "backup-id": group.backup_id(),
            "last-backup": info.backup_dir.backup_time().timestamp(),
            "backup-count": list.len() as u64,
            "files": info.files,
        }));
    }

    Ok(json!(groups))
}

fn list_snapshot_files (
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let store = tools::required_string_param(&param, "store")?;
    let backup_type = tools::required_string_param(&param, "backup-type")?;
    let backup_id = tools::required_string_param(&param, "backup-id")?;
    let backup_time = tools::required_integer_param(&param, "backup-time")?;

    let datastore = DataStore::lookup_datastore(store)?;
    let snapshot = BackupDir::new(backup_type, backup_id, backup_time);

    let mut files = read_backup_index(&datastore, &snapshot)?;

    let info = BackupInfo::new(&datastore.base_path(), snapshot)?;

    let file_set = files.as_array().unwrap().iter().fold(HashSet::new(), |mut acc, item| {
        acc.insert(item["filename"].as_str().unwrap().to_owned());
        acc
    });

    for file in info.files {
        if file_set.contains(&file) { continue; }
        files.as_array_mut().unwrap().push(json!({ "filename": file }));
    }

    Ok(files)
}

fn delete_snapshots (
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let store = tools::required_string_param(&param, "store")?;
    let backup_type = tools::required_string_param(&param, "backup-type")?;
    let backup_id = tools::required_string_param(&param, "backup-id")?;
    let backup_time = tools::required_integer_param(&param, "backup-time")?;

    let snapshot = BackupDir::new(backup_type, backup_id, backup_time);

    let datastore = DataStore::lookup_datastore(store)?;

    datastore.remove_backup_dir(&snapshot)?;

    Ok(Value::Null)
}

fn list_snapshots (
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

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

        let mut result_item = json!({
            "backup-type": group.backup_type(),
            "backup-id": group.backup_id(),
            "backup-time": info.backup_dir.backup_time().timestamp(),
            "files": info.files,
        });

        if let Ok(index) = read_backup_index(&datastore, &info.backup_dir) {
            let mut backup_size = 0;
            for item in index.as_array().unwrap().iter() {
                if let Some(item_size) = item["size"].as_u64() {
                    backup_size += item_size;
                }
            }
            result_item["size"] = backup_size.into();
        }

        snapshots.push(result_item);
    }

    Ok(json!(snapshots))
}

fn status(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let store = param["store"].as_str().unwrap();

    let datastore = DataStore::lookup_datastore(store)?;

    let base_path = datastore.base_path();

    let mut stat: libc::statfs64 = unsafe { std::mem::zeroed() };

    use nix::NixPath;

    let res = base_path.with_nix_path(|cstr| unsafe { libc::statfs64(cstr.as_ptr(), &mut stat) })?;
    nix::errno::Errno::result(res)?;

    let bsize = stat.f_bsize as u64;
    Ok(json!({
        "total": stat.f_blocks*bsize,
        "used": (stat.f_blocks-stat.f_bfree)*bsize,
        "avail": stat.f_bavail*bsize,
    }))
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

const API_METHOD_STATUS: ApiMethod = ApiMethod::new(
    &ApiHandler::Sync(&status),
    &ObjectSchema::new(
        "Get datastore status.",
        &add_common_prune_prameters!([],[
            ("store", false, &StringSchema::new("Datastore name.").schema()),
        ]),
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

    let group = BackupGroup::new(backup_type, backup_id);

    let datastore = DataStore::lookup_datastore(store)?;

    let prune_options = PruneOptions {
        keep_last: param["keep-last"].as_u64(),
        keep_daily: param["keep-daily"].as_u64(),
        keep_weekly: param["keep-weekly"].as_u64(),
        keep_monthly: param["keep-monthly"].as_u64(),
        keep_yearly: param["keep-yearly"].as_u64(),
    };

    let worker = WorkerTask::new("prune", Some(store.to_owned()), "root@pam", true)?;
    let result = try_block! {
        if !prune_options.keeps_something() {
            worker.log("No prune selection - keeping all files.");
            return Ok(());
        } else {
            worker.log(format!("Starting prune on store {}", store));
        }

        let list = group.list_backups(&datastore.base_path())?;

        let mut prune_info = compute_prune_info(list, &prune_options)?;

        prune_info.reverse(); // delete older snapshots first

        for (info, keep) in prune_info {
            if keep {
                worker.log(format!("keep {:?}", info.backup_dir.relative_path()));
            } else {
                worker.log(format!("remove {:?}", info.backup_dir.relative_path()));
                datastore.remove_backup_dir(&info.backup_dir)?;
            }
        }

        Ok(())
    };

    worker.log_result(&result);

    if let Err(err) = result {
        bail!("prune failed - {}", err);
    }

    Ok(json!(null))
}

const API_METHOD_PRUNE: ApiMethod = ApiMethod::new(
    &ApiHandler::Sync(&prune),
    &ObjectSchema::new(
        "Prune the datastore.",
        &add_common_prune_prameters!([
            ("backup-id", false, &BACKUP_ID_SCHEMA),
            ("backup-type", false, &BACKUP_TYPE_SCHEMA),
        ],[
            ("store", false, &StringSchema::new("Datastore name.").schema()),
        ])
    )
);

fn start_garbage_collection(
    param: Value,
    _info: &ApiMethod,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let store = param["store"].as_str().unwrap().to_string();

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

#[sortable]
pub const API_METHOD_START_GARBAGE_COLLECTION: ApiMethod = ApiMethod::new(
    &ApiHandler::Sync(&start_garbage_collection),
    &ObjectSchema::new(
        "Start garbage collection.",
        &sorted!([
            ("store", false, &StringSchema::new("Datastore name.").schema()),
        ])
    )
);

fn garbage_collection_status(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let store = param["store"].as_str().unwrap();

    let datastore = DataStore::lookup_datastore(&store)?;

    println!("Garbage collection status on store {}", store);

    let status = datastore.last_gc_status();

    Ok(serde_json::to_value(&status)?)
}

#[sortable]
pub const API_METHOD_GARBAGE_COLLECTION_STATUS: ApiMethod = ApiMethod::new(
    &ApiHandler::Sync(&garbage_collection_status),
    &ObjectSchema::new(
        "Garbage collection status.",
        &sorted!([
            ("store", false, &StringSchema::new("Datastore name.").schema()),
        ])
    )
);

fn get_datastore_list(
    _param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let config = datastore::config()?;

    Ok(config.convert_to_array("store"))
}


fn download_file(
    _parts: Parts,
    _req_body: Body,
    param: Value,
    _info: &ApiMethod,
    _rpcenv: Box<dyn RpcEnvironment>,
) -> ApiFuture {

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

        let payload = tokio::codec::FramedRead::new(file, tokio::codec::BytesCodec::new())
            .map_ok(|bytes| hyper::Chunk::from(bytes.freeze()));
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
pub const API_METHOD_DOWNLOAD_FILE: ApiMethod = ApiMethod::new(
    &ApiHandler::AsyncHttp(&download_file),
    &ObjectSchema::new(
        "Download single raw file from backup snapshot.",
        &sorted!([
            ("store", false, &StringSchema::new("Datastore name.").schema()),
            ("backup-type", false, &BACKUP_TYPE_SCHEMA),
            ("backup-id", false,  &BACKUP_ID_SCHEMA),
            ("backup-time", false, &BACKUP_TIME_SCHEMA),
            ("file-name", false, &StringSchema::new("Raw file name.")
             .format(&FILENAME_FORMAT)
             .schema()
            ),
        ]),
    )
);

fn upload_backup_log(
    _parts: Parts,
    req_body: Body,
    param: Value,
    _info: &ApiMethod,
    _rpcenv: Box<dyn RpcEnvironment>,
) -> ApiFuture {

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
        file_set_contents(&path, raw_data, None)?;

        // fixme: use correct formatter
        Ok(crate::server::formatter::json_response(Ok(Value::Null)))
    }.boxed()
}

#[sortable]
pub const API_METHOD_UPLOAD_BACKUP_LOG: ApiMethod = ApiMethod::new(
    &ApiHandler::AsyncHttp(&upload_backup_log),
    &ObjectSchema::new(
        "Download single raw file from backup snapshot.",
        &sorted!([
            ("store", false, &StringSchema::new("Datastore name.").schema()),
            ("backup-type", false, &BACKUP_TYPE_SCHEMA),
            ("backup-id", false, &BACKUP_ID_SCHEMA),
            ("backup-time", false, &BACKUP_TIME_SCHEMA),
        ]),
    )
);

const STORE_SCHEMA: Schema = StringSchema::new("Datastore name.").schema();

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
            .get(
                &ApiMethod::new(
                    &ApiHandler::Sync(&list_snapshot_files),
                    &ObjectSchema::new(
                        "List snapshot files.",
                        &sorted!([
                            ("store", false, &STORE_SCHEMA),
                            ("backup-type", false, &BACKUP_TYPE_SCHEMA),
                            ("backup-id", false, &BACKUP_ID_SCHEMA),
                            ("backup-time", false, &BACKUP_TIME_SCHEMA),
                        ]),
                    )
                )
            )
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
            .get(
                &ApiMethod::new(
                    &ApiHandler::Sync(&list_groups),
                    &ObjectSchema::new(
                        "List backup groups.",
                        &sorted!([ ("store", false, &STORE_SCHEMA) ]),
                    )
                )
            )
    ),
    (
        "prune",
        &Router::new()
            .post(&API_METHOD_PRUNE)
    ),
    (
        "snapshots",
        &Router::new()
            .get(
                &ApiMethod::new(
                    &ApiHandler::Sync(&list_snapshots),
                    &ObjectSchema::new(
                        "List backup groups.",
                        &sorted!([
                            ("store", false, &STORE_SCHEMA),
                            ("backup-type", true, &BACKUP_TYPE_SCHEMA),
                            ("backup-id", true, &BACKUP_ID_SCHEMA),
                        ]),
                    )
                )
            )
            .delete(
                &ApiMethod::new(
                    &ApiHandler::Sync(&delete_snapshots),
                    &ObjectSchema::new(
                        "Delete backup snapshot.",
                        &sorted!([
                            ("store", false, &STORE_SCHEMA),
                            ("backup-type", false, &BACKUP_TYPE_SCHEMA),
                            ("backup-id", false, &BACKUP_ID_SCHEMA),
                            ("backup-time", false, &BACKUP_TIME_SCHEMA),
                        ]),
                    )
                )
            )
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
