use failure::*;

use crate::api::schema::*;
use crate::api::router::*;
use crate::server::rest::*;
use serde_json::{json, Value};

use hyper::StatusCode;
use hyper::rt::{Future, Stream};

use crate::config::datastore;

use crate::backup::datastore::*;

mod upload_catar;

// this is just a test for mutability/mutex handling  - will remove later
fn start_garbage_collection(param: Value, _info: &ApiMethod) -> Result<Value, Error> {

    let name = param["name"].as_str().unwrap();

    let datastore = DataStore::lookup_datastore(name)?;

    println!("Starting garbage collection on store {}", name);

    datastore.garbage_collection()?;

    Ok(json!(null))
}

pub fn api_method_start_garbage_collection() -> ApiMethod {
    ApiMethod::new(
        start_garbage_collection,
        ObjectSchema::new("Start garbage collection.")
            .required("name", StringSchema::new("Datastore name."))
    )
}

fn garbage_collection_status(param: Value, _info: &ApiMethod) -> Result<Value, Error> {

    let name = param["name"].as_str().unwrap();

    println!("Garbage collection status on store {}", name);

    Ok(json!(null))

}

pub fn api_method_garbage_collection_status() -> ApiMethod {
    ApiMethod::new(
        garbage_collection_status,
        ObjectSchema::new("Garbage collection status.")
            .required("name", StringSchema::new("Datastore name."))
    )
}


fn get_datastore_list(_param: Value, _info: &ApiMethod) -> Result<Value, Error> {

    let config = datastore::config()?;

    Ok(config.convert_to_array("name"))
}


pub fn router() -> Router {

    let datastore_info = Router::new()
        .get(ApiMethod::new(
            |_,_| Ok(json!([
                {"subdir": "status"},
                {"subdir": "gc" }
            ])),
            ObjectSchema::new("Directory index.")
                .required("name", StringSchema::new("Datastore name.")))
        )
        .upload(upload_catar::api_method_upload_catar())
        .subdir(
            "gc",
            Router::new()
                .get(api_method_garbage_collection_status())
                .post(api_method_start_garbage_collection()));



    let route = Router::new()
        .get(ApiMethod::new(
            get_datastore_list,
            ObjectSchema::new("Directory index.")))
        .match_all("name", datastore_info);



    route
}
