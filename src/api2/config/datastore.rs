use std::path::PathBuf;

use failure::*;
use serde_json::Value;

use proxmox::api::{api, ApiMethod, Router, RpcEnvironment};

use crate::api2::types::*;
use crate::backup::*;
use crate::config::datastore;

#[api(
    input: {
        properties: {},
    },
    returns: {
        description: "List the configured datastores.",
        type: Array,
        items: {
            type: datastore::DataStoreConfig,
        },
    },
)]
/// List all datastores
pub fn list_datastores(
    _param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let config = datastore::config()?;

    Ok(config.convert_to_array("name"))
}

#[api(
    protected: true,
    input: {
        properties: {
            name: {
                schema: DATASTORE_SCHEMA,
            },
            comment: {
                optional: true,
                schema: datastore::COMMENT_SCHEMA,
            },
            path: {
                schema: datastore::DIR_NAME_SCHEMA,
            },
        },
    },
)]
/// Create new datastore config.
pub fn create_datastore(name: String, param: Value) -> Result<(), Error> {

    // fixme: locking ?

    let datastore: datastore::DataStoreConfig = serde_json::from_value(param.clone())?;

    let mut config = datastore::config()?;

    if let Some(_) = config.sections.get(&name) {
        bail!("datastore '{}' already exists.", name);
    }

    if let Some(ref comment) = datastore.comment {
        if comment.find(|c: char| c.is_control()).is_some() {
            bail!("comment must not contain control characters!");
        }
    }

    let path: PathBuf = datastore.path.clone().into();

    let backup_user = crate::backup::backup_user()?;
    let _store = ChunkStore::create(&name, path, backup_user.uid, backup_user.gid)?;

    config.set_data(&name, "datastore", &datastore)?;

    datastore::save_config(&config)?;

    Ok(())
}

#[api(
    protected: true,
    input: {
        properties: {
            name: {
                schema: DATASTORE_SCHEMA,
            },
        },
    },
)]
/// Remove a datastore configuration.
pub fn delete_datastore(name: String) -> Result<(), Error> {

    // fixme: locking ?
    // fixme: check digest ?

    let mut config = datastore::config()?;

    match config.sections.get(&name) {
        Some(_) => { config.sections.remove(&name); },
        None => bail!("datastore '{}' does not exist.", name),
    }

    datastore::save_config(&config)?;

    Ok(())
}

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_LIST_DATASTORES)
    .post(&API_METHOD_CREATE_DATASTORE)
    .delete(&API_METHOD_DELETE_DATASTORE);
