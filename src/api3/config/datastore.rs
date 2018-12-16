use failure::*;
//use std::collections::HashMap;

use crate::api::schema::*;
use crate::api::router::*;
use crate::backup::chunk_store::*;
use serde_json::{json, Value};
use std::path::PathBuf;

use crate::config::datastore;

pub fn get() -> ApiMethod {
    ApiMethod::new(
        get_datastore_list,
        ObjectSchema::new("Directory index."))
}

fn get_datastore_list(_param: Value, _info: &ApiMethod) -> Result<Value, Error> {

    let config = datastore::config()?;

    Ok(config.convert_to_array("name"))
}

pub fn post() -> ApiMethod {
    ApiMethod::new(
        create_datastore,
        ObjectSchema::new("Create new datastore.")
            .required("name", StringSchema::new("Datastore name."))
            .required("path", StringSchema::new("Directory path (must exist)."))
    )
}

fn create_datastore(param: Value, _info: &ApiMethod) -> Result<Value, Error> {

    // fixme: locking ?

    let mut config = datastore::config()?;

    let name = param["name"].as_str().unwrap();

    if let Some(_) = config.sections.get(name) {
        bail!("datastore '{}' already exists.", name);
    }

    let path: PathBuf = param["path"].as_str().unwrap().into();
    let _store = ChunkStore::create(path)?;

    let datastore = json!({
        "path": param["path"]
    });

    config.set_data(name, "datastore", datastore);

    datastore::save_config(&config)?;

    Ok(Value::Null)
}

pub fn delete() -> ApiMethod {
    ApiMethod::new(
        delete_datastore,
        ObjectSchema::new("Remove a datastore configuration.")
            .required("name", StringSchema::new("Datastore name.")))
}

fn delete_datastore(param: Value, _info: &ApiMethod) -> Result<Value, Error> {
    println!("This is a test {}", param);

    // fixme: locking ?
    // fixme: check digest ?

    let mut config = datastore::config()?;

    let name = param["name"].as_str().unwrap();

    match config.sections.get(name) {
        Some(_) => { config.sections.remove(name); },
        None => bail!("datastore '{}' does not exist.", name),
    }

    datastore::save_config(&config)?;

    Ok(Value::Null)
}

pub fn router() -> Router {

    let route = Router::new()
        .get(get())
        .post(post())
        .delete(delete());


    route
}
