use failure::*;

use std::collections::HashMap;
use lazy_static::lazy_static;
use std::sync::{Arc, Mutex};

use crate::api::schema::*;
use crate::api::router::*;
use serde_json::{json, Value};

pub mod config;
mod version;

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

fn test_sync_api_handler(param: Value, _info: &ApiMethod) -> Result<Value, Error> {
    println!("This is a test {}", param);

   // let force: Option<bool> = Some(false);

    //if let Some(force) = param.force {
    //}

    let _force =  param["force"].as_bool()
        .ok_or_else(|| format_err!("missing parameter 'force'"))?;

    if let Some(_force) = param["force"].as_bool() {
    }

    Ok(json!(null))
}

pub fn router() -> Router {

    let route4 = Router::new()
        .get(ApiMethod::new(
            |param, _info| {
                println!("This is a clousure handler: {}", param);

                Ok(json!(null))
            },
            ObjectSchema::new("Another Endpoint."))
             .returns(Schema::Null));


    let nodeinfo = Router::new()
        .get(ApiMethod::new(
            test_sync_api_handler,
            ObjectSchema::new("This is a simple test.")
                .optional("force", BooleanSchema::new("Test for boolean options")))
        )
        .subdir("subdir3", route4);

    let nodes = Router::new()
        .match_all("node", nodeinfo);


    let route = Router::new()
        .get(ApiMethod::new(
            |_,_| Ok(json!([
                {"subdir": "config"},
                {"subdir": "version"},
                {"subdir": "nodes"}
            ])),
            ObjectSchema::new("Directory index.")))
        .subdir("config", config::router())
        .subdir("version", version::router())
        .subdir("nodes", nodes);

    route
}
