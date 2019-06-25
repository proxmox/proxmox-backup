use failure::*;

use crate::tools;
use crate::api_schema::*;
use crate::api_schema::router::*;
//use crate::server::rest::*;
use serde_json::{json, Value};
use std::collections::{HashSet, HashMap};
use chrono::{DateTime, Datelike, Local};
use std::path::PathBuf;
use std::sync::Arc;

//use hyper::StatusCode;
//use hyper::rt::{Future, Stream};

use crate::config::datastore;

use crate::backup::*;
use crate::server::WorkerTask;

mod pxar;

fn group_backups(backup_list: Vec<BackupInfo>) -> HashMap<String, Vec<BackupInfo>> {

    let mut group_hash = HashMap::new();

    for info in backup_list {
        let group_id = info.backup_dir.group().group_path().to_str().unwrap().to_owned();
        let time_list = group_hash.entry(group_id).or_insert(vec![]);
        time_list.push(info);
    }

    group_hash
}

fn mark_selections<F: Fn(DateTime<Local>, &BackupInfo) -> String> (
    mark: &mut HashSet<PathBuf>,
    list: &Vec<BackupInfo>,
    keep: usize,
    select_id: F,
){
    let mut hash = HashSet::new();
    for info in list {
        let local_time = info.backup_dir.backup_time().with_timezone(&Local);
        if hash.len() >= keep as usize { break; }
        let backup_id = info.backup_dir.relative_path();
        let sel_id: String = select_id(local_time, &info);
        if !hash.contains(&sel_id) {
            hash.insert(sel_id);
            //println!(" KEEP ID {} {}", backup_id, local_time.format("%c"));
            mark.insert(backup_id);
        }
    }
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

    let snapshot = BackupDir::new(backup_type, backup_id, backup_time);

    let datastore = DataStore::lookup_datastore(store)?;

    let path = datastore.base_path();
    let files = BackupInfo::list_files(&path, &snapshot)?;

    Ok(json!(files))
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
    let backup_type = tools::required_string_param(&param, "backup-type")?;
    let backup_id = tools::required_string_param(&param, "backup-id")?;

    let group = BackupGroup::new(backup_type, backup_id);

    let datastore = DataStore::lookup_datastore(store)?;

    let base_path = datastore.base_path();

    let backup_list = group.list_backups(&base_path)?;

    let mut snapshots = vec![];

    for info in backup_list {
        snapshots.push(json!({
            "backup-type": group.backup_type(),
            "backup-id": group.backup_id(),
            "backup-time": info.backup_dir.backup_time().timestamp(),
            "files": info.files,
        }));
    }

    Ok(json!(snapshots))
}

fn prune(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let store = param["store"].as_str().unwrap();

    let datastore = DataStore::lookup_datastore(store)?;

    let mut keep_all = true;

    for opt in &["keep-last", "keep-daily", "keep-weekly", "keep-weekly", "keep-yearly"] {
        if !param[opt].is_null() {
            keep_all = false;
            break;
        }
    }

    let worker = WorkerTask::new("prune", Some(store.to_owned()), "root@pam", true)?;
    let result = try_block! {
        if keep_all {
            worker.log("No selection - keeping all files.");
            return Ok(());
        } else {
            worker.log(format!("Starting prune on store {}", store));
        }

        let backup_list = BackupInfo::list_backups(&datastore.base_path())?;

        let group_hash = group_backups(backup_list);

        for (_group_id, mut list) in group_hash {

            let mut mark = HashSet::new();

            BackupInfo::sort_list(&mut list, false);

            if let Some(keep_last) = param["keep-last"].as_u64() {
                list.iter().take(keep_last as usize).for_each(|info| {
                    mark.insert(info.backup_dir.relative_path());
                });
            }

            if let Some(keep_daily) = param["keep-daily"].as_u64() {
                mark_selections(&mut mark, &list, keep_daily as usize, |local_time, _info| {
                    format!("{}/{}/{}", local_time.year(), local_time.month(), local_time.day())
                });
            }

            if let Some(keep_weekly) = param["keep-weekly"].as_u64() {
                mark_selections(&mut mark, &list, keep_weekly as usize, |local_time, _info| {
                    format!("{}/{}", local_time.year(), local_time.iso_week().week())
                });
            }

            if let Some(keep_monthly) = param["keep-monthly"].as_u64() {
                mark_selections(&mut mark, &list, keep_monthly as usize, |local_time, _info| {
                    format!("{}/{}", local_time.year(), local_time.month())
                });
            }

            if let Some(keep_yearly) = param["keep-yearly"].as_u64() {
                mark_selections(&mut mark, &list, keep_yearly as usize, |local_time, _info| {
                    format!("{}/{}", local_time.year(), local_time.year())
                });
            }

            let mut remove_list: Vec<BackupInfo> = list.into_iter()
                .filter(|info| !mark.contains(&info.backup_dir.relative_path())).collect();

            BackupInfo::sort_list(&mut remove_list, true);

            for info in remove_list {
                worker.log(format!("remove {:?}", info.backup_dir));
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

pub fn add_common_prune_prameters(schema: ObjectSchema) -> ObjectSchema  {

    schema
        .optional(
            "keep-last",
            IntegerSchema::new("Number of backups to keep.")
                .minimum(1)
        )
        .optional(
            "keep-daily",
            IntegerSchema::new("Number of daily backups to keep.")
                .minimum(1)
        )
        .optional(
            "keep-weekly",
            IntegerSchema::new("Number of weekly backups to keep.")
                .minimum(1)
        )
        .optional(
            "keep-monthly",
            IntegerSchema::new("Number of monthly backups to keep.")
                .minimum(1)
        )
        .optional(
            "keep-yearly",
            IntegerSchema::new("Number of yearly backups to keep.")
                .minimum(1)
        )
}

fn api_method_prune() -> ApiMethod {
    ApiMethod::new(
        prune,
        add_common_prune_prameters(
            ObjectSchema::new("Prune the datastore.")
                .required(
                    "store",
                    StringSchema::new("Datastore name.")
                )
        )
    )
}

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

pub fn api_method_start_garbage_collection() -> ApiMethod {
    ApiMethod::new(
        start_garbage_collection,
        ObjectSchema::new("Start garbage collection.")
            .required("store", StringSchema::new("Datastore name."))
    )
}

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

pub fn api_method_garbage_collection_status() -> ApiMethod {
    ApiMethod::new(
        garbage_collection_status,
        ObjectSchema::new("Garbage collection status.")
            .required("store", StringSchema::new("Datastore name."))
    )
}

fn get_backup_list(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    //let config = datastore::config()?;

    let store = param["store"].as_str().unwrap();

    let datastore = DataStore::lookup_datastore(store)?;

    let mut list = vec![];

    let backup_list = BackupInfo::list_backups(&datastore.base_path())?;

    for info in backup_list {
        list.push(json!({
            "backup-type": info.backup_dir.group().backup_type(),
            "backup-id": info.backup_dir.group().backup_id(),
            "backup-time": info.backup_dir.backup_time().timestamp(),
            "files": info.files,
        }));
    }

    let result = json!(list);

    Ok(result)
}

fn get_datastore_list(
    _param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let config = datastore::config()?;

    Ok(config.convert_to_array("store"))
}


pub fn router() -> Router {

    let store_schema: Arc<Schema> = Arc::new(
        StringSchema::new("Datastore name.").into()
    );

    let datastore_info = Router::new()
        .subdir(
            "backups",
            Router::new()
                .get(ApiMethod::new(
                    get_backup_list,
                    ObjectSchema::new("List backups.")
                        .required("store", store_schema.clone()))))
        .subdir(
            "pxar",
            Router::new()
                .download(pxar::api_method_download_pxar())
        )
        .subdir(
            "gc",
            Router::new()
                .get(api_method_garbage_collection_status())
                .post(api_method_start_garbage_collection()))
        .subdir(
            "files",
            Router::new()
                .get(
                    ApiMethod::new(
                        list_snapshot_files,
                        ObjectSchema::new("List snapshot files.")
                            .required("store", store_schema.clone())
                            .required("backup-type", StringSchema::new("Backup type."))
                            .required("backup-id", StringSchema::new("Backup ID."))
                            .required("backup-time", IntegerSchema::new("Backup time (Unix epoch.)")
                                      .minimum(1547797308))
                    )
                )
        )
        .subdir(
            "groups",
            Router::new()
                .get(ApiMethod::new(
                    list_groups,
                    ObjectSchema::new("List backup groups.")
                        .required("store", store_schema.clone()))))
        .subdir(
            "snapshots",
            Router::new()
                .get(
                    ApiMethod::new(
                        list_snapshots,
                        ObjectSchema::new("List backup groups.")
                            .required("store", store_schema.clone())
                            .required("backup-type", StringSchema::new("Backup type."))
                            .required("backup-id", StringSchema::new("Backup ID."))
                    )
                )
                .delete(
                    ApiMethod::new(
                        delete_snapshots,
                        ObjectSchema::new("Delete backup snapshot.")
                            .required("store", store_schema.clone())
                            .required("backup-type", StringSchema::new("Backup type."))
                            .required("backup-id", StringSchema::new("Backup ID."))
                            .required("backup-time", IntegerSchema::new("Backup time (Unix epoch.)")
                                      .minimum(1547797308))
                    )
                )
        )
        .subdir(
            "prune",
            Router::new()
                .post(api_method_prune())
        )
        .list_subdirs();



    let route = Router::new()
        .get(ApiMethod::new(
            get_datastore_list,
            ObjectSchema::new("Directory index.")))
        .match_all("store", datastore_info);



    route
}
