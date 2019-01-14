use failure::*;

use crate::api::schema::*;
use crate::api::router::*;
use crate::server::rest::*;
use serde_json::{json, Value};

use hyper::StatusCode;
use hyper::rt::{Future, Stream};

use crate::config::datastore;

use crate::backup::datastore::*;

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

fn upload_catar(req_body: hyper::Body, param: Value, _info: &ApiUploadMethod) -> BoxFut {

    let name = param["name"].as_str().unwrap();

    println!("Upload .catar to {}", name);

    let resp = req_body
        .map_err(|err| http_err!(BAD_REQUEST, format!("Promlems reading request body: {}", err)))
        .for_each(|chunk| {
             println!("UPLOAD Chunk {}", chunk.len());
            Ok(())
        })
        .and_then(|()| {
            println!("UPLOAD DATA Sucessful");

            let response = http::Response::builder()
                .status(200)
                .body(hyper::Body::empty())
                .unwrap();

            Ok(response)
        });

    Box::new(resp)
}

fn api_method_upload_catar() -> ApiUploadMethod {
    ApiUploadMethod::new(
        upload_catar,
        ObjectSchema::new("Upload .catar backup file.")
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
        .upload(api_method_upload_catar())
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
