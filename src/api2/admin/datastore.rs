use failure::*;

use crate::api_schema::schema::*;
use crate::api_schema::router::*;
//use crate::server::rest::*;
use serde_json::{json, Value};

//use hyper::StatusCode;
//use hyper::rt::{Future, Stream};

use crate::config::datastore;

use crate::backup::*;

mod catar;

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
            "backup_type": info.backup_type,
            "backup_id": info.backup_id,
            "backup_time": info.backup_time.timestamp(),
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
                {"subdir": "status"},
                {"subdir": "gc" }
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
                .post(api_method_start_garbage_collection()));



    let route = Router::new()
        .get(ApiMethod::new(
            get_datastore_list,
            ObjectSchema::new("Directory index.")))
        .match_all("store", datastore_info);



    route
}
