use failure::*;

use std::collections::HashMap;
use lazy_static::lazy_static;
use std::sync::{Arc, Mutex};

use crate::api::schema::*;
use crate::api::router::*;
use serde_json::{json, Value};

use crate::config::datastore;

use crate::backup::datastore::*;

lazy_static!{
    static ref datastore_map: Mutex<HashMap<String, Arc<DataStore>>> =  Mutex::new(HashMap::new());
}

fn lookup_datastore(name: &str) -> Result<Arc<DataStore>, Error> {

    let mut map = datastore_map.lock().unwrap();

    if let Some(datastore) = map.get(name) {
        return Ok(datastore.clone());
    }

    if let Ok(datastore) = DataStore::open(name)  {
        let datastore = Arc::new(datastore);
        map.insert(name.to_string(), datastore.clone());
        return Ok(datastore);
    }

    bail!("store not found");
}

// this is just a test for mutability/mutex handling  - will remove later
fn start_garbage_collection(param: Value, _info: &ApiMethod) -> Result<Value, Error> {

    let name = param["name"].as_str().unwrap();

    let datastore = lookup_datastore(name)?;

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
