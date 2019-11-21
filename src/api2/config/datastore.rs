use failure::*;
//use std::collections::HashMap;

use crate::api_schema::*;
use crate::api_schema::router::*;
use crate::backup::*;
use serde_json::{json, Value};
use std::path::PathBuf;

use crate::config::datastore;

pub const GET: ApiMethod = ApiMethod::new(
    &ApiHandler::Sync(&get_datastore_list),
    &ObjectSchema::new("Directory index.", &[])
);

fn get_datastore_list(
    _param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let config = datastore::config()?;

    Ok(config.convert_to_array("name"))
}

pub const POST: ApiMethod = ApiMethod::new(
    &ApiHandler::Sync(&create_datastore),
    &ObjectSchema::new(
        "Create new datastore.",
        &[
            ("name", false, &StringSchema::new("Datastore name.").schema()),
            ("path", false, &StringSchema::new("Directory path (must exist).").schema()),
        ],
    )       
);

fn create_datastore(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    // fixme: locking ?

    let mut config = datastore::config()?;

    let name = param["name"].as_str().unwrap();

    if let Some(_) = config.sections.get(name) {
        bail!("datastore '{}' already exists.", name);
    }

    let path: PathBuf = param["path"].as_str().unwrap().into();
    let _store = ChunkStore::create(name, path)?;

    let datastore = json!({
        "path": param["path"]
    });

    config.set_data(name, "datastore", datastore);

    datastore::save_config(&config)?;

    Ok(Value::Null)
}

pub const DELETE: ApiMethod = ApiMethod::new(
    &ApiHandler::Sync(&delete_datastore),
    &ObjectSchema::new(
        "Remove a datastore configuration.",
        &[
            ("name", false, &StringSchema::new("Datastore name.").schema()),
        ],
    )
);

fn delete_datastore(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {
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

pub const ROUTER: Router = Router::new()
    .get(&GET)
    .post(&POST)
    .delete(&DELETE);
