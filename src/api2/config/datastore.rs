use std::path::PathBuf;

use failure::*;
use serde_json::{json, Value};

use proxmox::api::{ApiHandler, ApiMethod, Router, RpcEnvironment};
use proxmox::api::schema::*;

use crate::api2::types::*;
use crate::backup::*;
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
            ("comment", true, &StringSchema::new("Comment for this Datastore").schema()),
            ("name", false, &DATASTORE_SCHEMA),
            ("path", false, &StringSchema::new("Directory path. The directory path is created if it does not already exist.").schema()),
        ],
    )
).protected(true);

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

    if param["comment"].as_str().unwrap().find(|c: char| c.is_control()) != None {
        bail!("comment must not contain control characters!");
    }

    let path: PathBuf = param["path"].as_str().unwrap().into();
    let backup_user = crate::backup::backup_user()?;
    let _store = ChunkStore::create(name, path, backup_user)?;

    let datastore = json!({
        "path": param["path"],
        "comment": param["comment"],
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
            ("name", false, &DATASTORE_SCHEMA),
        ],
    )
).protected(true);

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
