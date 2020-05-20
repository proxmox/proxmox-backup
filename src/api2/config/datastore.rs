use std::path::PathBuf;

use anyhow::{bail, Error};
use serde_json::Value;
use ::serde::{Deserialize, Serialize};

use proxmox::api::{api, Router, RpcEnvironment, Permission};

use crate::api2::types::*;
use crate::backup::*;
use crate::config::datastore::{self, DataStoreConfig, DIR_NAME_SCHEMA};
use crate::config::acl::{PRIV_DATASTORE_AUDIT, PRIV_DATASTORE_MODIFY};

#[api(
    input: {
        properties: {},
    },
    returns: {
        description: "List the configured datastores (with config digest).",
        type: Array,
        items: { type: datastore::DataStoreConfig },
    },
    access: {
        permission: &Permission::Privilege(&["datastore"], PRIV_DATASTORE_AUDIT, false),
    },
)]
/// List all datastores
pub fn list_datastores(
    _param: Value,
    mut rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<DataStoreConfig>, Error> {

    let (config, digest) = datastore::config()?;

    let list = config.convert_to_typed_array("datastore")?;

    rpcenv["digest"] = proxmox::tools::digest_to_hex(&digest).into();

    Ok(list)
}


// fixme: impl. const fn get_object_schema(datastore::DataStoreConfig::API_SCHEMA),
// but this need support for match inside const fn
// see: https://github.com/rust-lang/rust/issues/49146

#[api(
    protected: true,
    input: {
        properties: {
            name: {
                schema: DATASTORE_SCHEMA,
            },
            path: {
                schema: DIR_NAME_SCHEMA,
            },
            comment: {
                optional: true,
                schema: SINGLE_LINE_COMMENT_SCHEMA,
            },
            "gc-schedule": {
                optional: true,
                schema: GC_SCHEDULE_SCHEMA,
            },
            "prune-schedule": {
                optional: true,
                schema: PRUNE_SCHEDULE_SCHEMA,
            },
            "keep-last": {
                optional: true,
                schema: PRUNE_SCHEMA_KEEP_LAST,
            },
            "keep-hourly": {
                optional: true,
                schema: PRUNE_SCHEMA_KEEP_HOURLY,
            },
            "keep-daily": {
                optional: true,
                schema: PRUNE_SCHEMA_KEEP_DAILY,
            },
            "keep-weekly": {
                optional: true,
                schema: PRUNE_SCHEMA_KEEP_WEEKLY,
            },
            "keep-monthly": {
                optional: true,
                schema: PRUNE_SCHEMA_KEEP_MONTHLY,
            },
            "keep-yearly": {
                optional: true,
                schema: PRUNE_SCHEMA_KEEP_YEARLY,
            },
        },
    },
    access: {
        permission: &Permission::Privilege(&["datastore"], PRIV_DATASTORE_MODIFY, false),
    },
)]
/// Create new datastore config.
pub fn create_datastore(param: Value) -> Result<(), Error> {

    let _lock = crate::tools::open_file_locked(datastore::DATASTORE_CFG_LOCKFILE, std::time::Duration::new(10, 0))?;

    let datastore: datastore::DataStoreConfig = serde_json::from_value(param.clone())?;

    let (mut config, _digest) = datastore::config()?;

    if let Some(_) = config.sections.get(&datastore.name) {
        bail!("datastore '{}' already exists.", datastore.name);
    }

    let path: PathBuf = datastore.path.clone().into();

    let backup_user = crate::backup::backup_user()?;
    let _store = ChunkStore::create(&datastore.name, path, backup_user.uid, backup_user.gid)?;

    config.set_data(&datastore.name, "datastore", &datastore)?;

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
pub fn read_datastore(
    name: String,
    mut rpcenv: &mut dyn RpcEnvironment,
) -> Result<DataStoreConfig, Error> {
    let (config, digest) = datastore::config()?;

    let store_config = config.lookup("datastore", &name)?;
    rpcenv["digest"] = proxmox::tools::digest_to_hex(&digest).into();

    Ok(store_config)
}

#[api()]
#[derive(Serialize, Deserialize)]
#[serde(rename_all="kebab-case")]
#[allow(non_camel_case_types)]
/// Deletable property name
pub enum DeletableProperty {
    /// Delete the comment property.
    comment,
    /// Delete the garbage collection schedule.
    gc_schedule,
    /// Delete the prune job schedule.
    prune_schedule,
    /// Delete the keep-last property
    keep_last,
    /// Delete the keep-hourly property
    keep_hourly,
    /// Delete the keep-daily property
    keep_daily,
    /// Delete the keep-weekly property
    keep_weekly,
    /// Delete the keep-monthly property
    keep_monthly,
    /// Delete the keep-yearly property
    keep_yearly,
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
            "gc-schedule": {
                optional: true,
                schema: GC_SCHEDULE_SCHEMA,
            },
            "prune-schedule": {
                optional: true,
                schema: PRUNE_SCHEDULE_SCHEMA,
            },
            "keep-last": {
                optional: true,
                schema: PRUNE_SCHEMA_KEEP_LAST,
            },
            "keep-hourly": {
                optional: true,
                schema: PRUNE_SCHEMA_KEEP_HOURLY,
            },
            "keep-daily": {
                optional: true,
                schema: PRUNE_SCHEMA_KEEP_DAILY,
            },
            "keep-weekly": {
                optional: true,
                schema: PRUNE_SCHEMA_KEEP_WEEKLY,
            },
            "keep-monthly": {
                optional: true,
                schema: PRUNE_SCHEMA_KEEP_MONTHLY,
            },
            "keep-yearly": {
                optional: true,
                schema: PRUNE_SCHEMA_KEEP_YEARLY,
            },
            delete: {
                description: "List of properties to delete.",
                type: Array,
                optional: true,
                items: {
                    type: DeletableProperty,
                }
            },
            digest: {
                optional: true,
                schema: PROXMOX_CONFIG_DIGEST_SCHEMA,
            },
        },
    },
    access: {
        permission: &Permission::Privilege(&["datastore", "{name}"], PRIV_DATASTORE_MODIFY, false),
    },
)]
/// Update datastore config.
pub fn update_datastore(
    name: String,
    comment: Option<String>,
    gc_schedule: Option<String>,
    prune_schedule: Option<String>,
    keep_last: Option<u64>,
    keep_hourly: Option<u64>,
    keep_daily: Option<u64>,
    keep_weekly: Option<u64>,
    keep_monthly: Option<u64>,
    keep_yearly: Option<u64>,
    delete: Option<Vec<DeletableProperty>>,
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

     if let Some(delete) = delete {
        for delete_prop in delete {
            match delete_prop {
                DeletableProperty::comment => { data.comment = None; },
                DeletableProperty::gc_schedule => { data.gc_schedule = None; },
                DeletableProperty::prune_schedule => { data.prune_schedule = None; },
                DeletableProperty::keep_last => { data.keep_last = None; },
                DeletableProperty::keep_hourly => { data.keep_hourly = None; },
                DeletableProperty::keep_daily => { data.keep_daily = None; },
                DeletableProperty::keep_weekly => { data.keep_weekly = None; },
                DeletableProperty::keep_monthly => { data.keep_monthly = None; },
                DeletableProperty::keep_yearly => { data.keep_yearly = None; },
            }
        }
    }

    if let Some(comment) = comment {
        let comment = comment.trim().to_string();
        if comment.is_empty() {
            data.comment = None;
        } else {
            data.comment = Some(comment);
        }
    }

    if gc_schedule.is_some() { data.gc_schedule = gc_schedule; }
    if prune_schedule.is_some() { data.prune_schedule = prune_schedule; }

    if keep_last.is_some() { data.keep_last = keep_last; }
    if keep_hourly.is_some() { data.keep_hourly = keep_hourly; }
    if keep_daily.is_some() { data.keep_daily = keep_daily; }
    if keep_weekly.is_some() { data.keep_weekly = keep_weekly; }
    if keep_monthly.is_some() { data.keep_monthly = keep_monthly; }
    if keep_yearly.is_some() { data.keep_yearly = keep_yearly; }

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
        permission: &Permission::Privilege(&["datastore", "{name}"], PRIV_DATASTORE_MODIFY, false),
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
