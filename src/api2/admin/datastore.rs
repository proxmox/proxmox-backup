use failure::*;

use crate::api_schema::*;
use crate::api_schema::router::*;
//use crate::server::rest::*;
use serde_json::{json, Value};
use std::collections::{HashSet, HashMap};
use chrono::{DateTime, Datelike, Local};
use std::path::PathBuf;

//use hyper::StatusCode;
//use hyper::rt::{Future, Stream};

use crate::config::datastore;

use crate::backup::*;

mod catar;

fn group_backups(backup_list: Vec<BackupInfo>) -> HashMap<String, Vec<BackupInfo>> {

    let mut group_hash = HashMap::new();

    for info in backup_list {
        let group_id = format!("{}/{}", info.backup_dir.group.backup_type, info.backup_dir.group.backup_id);
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
        let local_time = info.backup_dir.backup_time.with_timezone(&Local);
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


fn prune(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut RpcEnvironment,
) -> Result<Value, Error> {

    let store = param["store"].as_str().unwrap();

    let datastore = DataStore::lookup_datastore(store)?;

    println!("Starting prune on store {}", store);

    let backup_list =  datastore.list_backups()?;

    let group_hash = group_backups(backup_list);

    for (_group_id, mut list) in group_hash {

        let mut mark = HashSet::new();

        list.sort_unstable_by(|a, b| b.backup_dir.backup_time.cmp(&a.backup_dir.backup_time)); // new backups first

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

        let mut remove_list: Vec<&BackupInfo> = list.iter()
            .filter(|info| !mark.contains(&info.backup_dir.relative_path())).collect();

        remove_list.sort_unstable_by(|a, b| a.backup_dir.backup_time.cmp(&b.backup_dir.backup_time)); // oldest backups first

        for info in remove_list {
            datastore.remove_backup_dir(&info.backup_dir)?;
        }
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

// this is just a test for mutability/mutex handling  - will remove later
fn start_garbage_collection(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut RpcEnvironment,
) -> Result<Value, Error> {

    let store = param["store"].as_str().unwrap();

    let datastore = DataStore::lookup_datastore(store)?;

    println!("Starting garbage collection on store {}", store);

    datastore.garbage_collection()?;

    Ok(json!(null))
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
    _rpcenv: &mut RpcEnvironment,
) -> Result<Value, Error> {

    let store = param["store"].as_str().unwrap();

    println!("Garbage collection status on store {}", store);

    Ok(json!(null))

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
    _rpcenv: &mut RpcEnvironment,
) -> Result<Value, Error> {

    //let config = datastore::config()?;

    let store = param["store"].as_str().unwrap();

    let datastore = DataStore::lookup_datastore(store)?;

    let mut list = vec![];

    for info in datastore.list_backups()? {
        list.push(json!({
            "backup_type": info.backup_dir.group.backup_type,
            "backup_id": info.backup_dir.group.backup_id,
            "backup_time": info.backup_dir.backup_time.timestamp(),
            "files": info.files,
        }));
    }

    let result = json!(list);

    Ok(result)
}

fn get_datastore_list(
    _param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut RpcEnvironment,
) -> Result<Value, Error> {

    let config = datastore::config()?;

    Ok(config.convert_to_array("store"))
}


pub fn router() -> Router {

    let datastore_info = Router::new()
        .get(ApiMethod::new(
            |_,_,_| Ok(json!([
                {"subdir": "backups" },
                {"subdir": "catar" },
                {"subdir": "gc" },
                {"subdir": "status" },
                {"subdir": "prune" },
           ])),
            ObjectSchema::new("Directory index.")
                .required("store", StringSchema::new("Datastore name.")))
        )
        .subdir(
            "backups",
            Router::new()
                .get(ApiMethod::new(
                    get_backup_list,
                    ObjectSchema::new("List backups.")
                        .required("store", StringSchema::new("Datastore name.")))))
        .subdir(
            "catar",
            Router::new()
                .download(catar::api_method_download_catar())
                .upload(catar::api_method_upload_catar()))
        .subdir(
            "gc",
            Router::new()
                .get(api_method_garbage_collection_status())
                .post(api_method_start_garbage_collection()))
        .subdir(
            "prune",
            Router::new()
                .post(api_method_prune()));



    let route = Router::new()
        .get(ApiMethod::new(
            get_datastore_list,
            ObjectSchema::new("Directory index.")))
        .match_all("store", datastore_info);



    route
}
