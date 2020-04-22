use std::path::PathBuf;

use anyhow::{bail, Error};
use serde_json::Value;

use proxmox::api::{api, ApiMethod, Router, RpcEnvironment, Permission};

use crate::api2::types::*;
use crate::backup::*;
use crate::config::datastore;
use crate::config::acl::{PRIV_DATASTORE_AUDIT, PRIV_DATASTORE_ALLOCATE};

#[api(
    input: {
        properties: {},
    },
    returns: {
        description: "List the configured datastores (with config digest).",
        type: Array,
        items: {
            type: datastore::DataStoreConfig,
        },
    },
    access: {
        permission: &Permission::Privilege(&["datastore"], PRIV_DATASTORE_AUDIT, false),
    },
)]
/// List all datastores
pub fn list_datastores(
    _param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let (config, digest) = datastore::config()?;

    Ok(config.convert_to_array("name", Some(&digest), &[]))
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
                schema: SINGLE_LINE_COMMENT_SCHEMA,
            },
            path: {
                schema: datastore::DIR_NAME_SCHEMA,
            },
        },
    },
    access: {
        permission: &Permission::Privilege(&["datastore"], PRIV_DATASTORE_ALLOCATE, false),
    },
)]
/// Create new datastore config.
pub fn create_datastore(name: String, param: Value) -> Result<(), Error> {

    let _lock = crate::tools::open_file_locked(datastore::DATASTORE_CFG_LOCKFILE, std::time::Duration::new(10, 0))?;

    let datastore: datastore::DataStoreConfig = serde_json::from_value(param.clone())?;

    let (mut config, _digest) = datastore::config()?;

    if let Some(_) = config.sections.get(&name) {
        bail!("datastore '{}' already exists.", name);
    }

    let path: PathBuf = datastore.path.clone().into();

    let backup_user = crate::backup::backup_user()?;
    let _store = ChunkStore::create(&name, path, backup_user.uid, backup_user.gid)?;

    config.set_data(&name, "datastore", &datastore)?;

    datastore::save_config(&config)?;

    Ok(())
}

#[api(
   input: {
        properties: {
            name: {
                schema: DATASTORE_SCHEMA,
            },
        },
    },
    returns: {
        description: "The datastore configuration (with config digest).",
        type: datastore::DataStoreConfig,
    },
    access: {
        permission: &Permission::Privilege(&["datastore", "{name}"], PRIV_DATASTORE_AUDIT, false),
    },
)]
/// Read a datastore configuration.
pub fn read_datastore(name: String) -> Result<Value, Error> {
    let (config, digest) = datastore::config()?;
    let mut data = config.lookup_json("datastore", &name)?;
    data.as_object_mut().unwrap()
        .insert("digest".into(), proxmox::tools::digest_to_hex(&digest).into());
    Ok(data)
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
                schema: SINGLE_LINE_COMMENT_SCHEMA,
            },
            digest: {
                optional: true,
                schema: PROXMOX_CONFIG_DIGEST_SCHEMA,
            },
        },
    },
    access: {
        permission: &Permission::Privilege(&["datastore", "{name}"], PRIV_DATASTORE_ALLOCATE, false),
    },
)]
/// Update datastore config.
pub fn update_datastore(
    name: String,
    comment: Option<String>,
    digest: Option<String>,
) -> Result<(), Error> {

    let _lock = crate::tools::open_file_locked(datastore::DATASTORE_CFG_LOCKFILE, std::time::Duration::new(10, 0))?;

    // pass/compare digest
    let (mut config, expected_digest) = datastore::config()?;

    if let Some(ref digest) = digest {
        let digest = proxmox::tools::hex_to_digest(digest)?;
        crate::tools::detect_modified_configuration_file(&digest, &expected_digest)?;
    }

    let mut data: datastore::DataStoreConfig = config.lookup("datastore", &name)?;

    if let Some(comment) = comment {
        let comment = comment.trim().to_string();
        if comment.is_empty() {
            data.comment = None;
        } else {
            data.comment = Some(comment);
        }
    }

    config.set_data(&name, "datastore", &data)?;

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
            digest: {
                optional: true,
                schema: PROXMOX_CONFIG_DIGEST_SCHEMA,
            },
        },
    },
    access: {
        permission: &Permission::Privilege(&["datastore", "{name}"], PRIV_DATASTORE_ALLOCATE, false),
    },
)]
/// Remove a datastore configuration.
pub fn delete_datastore(name: String, digest: Option<String>) -> Result<(), Error> {

    let _lock = crate::tools::open_file_locked(datastore::DATASTORE_CFG_LOCKFILE, std::time::Duration::new(10, 0))?;

    let (mut config, expected_digest) = datastore::config()?;

    if let Some(ref digest) = digest {
        let digest = proxmox::tools::hex_to_digest(digest)?;
        crate::tools::detect_modified_configuration_file(&digest, &expected_digest)?;
    }

    match config.sections.get(&name) {
        Some(_) => { config.sections.remove(&name); },
        None => bail!("datastore '{}' does not exist.", name),
    }

    datastore::save_config(&config)?;

    Ok(())
}

const ITEM_ROUTER: Router = Router::new()
    .get(&API_METHOD_READ_DATASTORE)
    .put(&API_METHOD_UPDATE_DATASTORE)
    .delete(&API_METHOD_DELETE_DATASTORE);

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_LIST_DATASTORES)
    .post(&API_METHOD_CREATE_DATASTORE)
    .match_all("name", &ITEM_ROUTER);
