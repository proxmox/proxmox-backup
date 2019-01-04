use failure::*;

use crate::api::schema::*;
use crate::api::router::*;
use serde_json::{json, Value};

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
        .subdir(
            "gc",
            Router::new()
                .post(ApiMethod::new(
                    start_garbage_collection,
                    ObjectSchema::new("Start garbage collection.")
                        .required("name", StringSchema::new("Datastore name."))
                )
                ));
               


    let route = Router::new()
        .get(ApiMethod::new(
            get_datastore_list,
            ObjectSchema::new("Directory index.")))
        .match_all("name", datastore_info);



    route
}
