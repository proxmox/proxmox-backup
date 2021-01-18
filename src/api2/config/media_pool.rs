use anyhow::{bail, Error};
use ::serde::{Deserialize, Serialize};

use proxmox::{
    api::{
        api,
        Router,
        RpcEnvironment,
    },
};

use crate::{
    api2::types::{
        DRIVE_NAME_SCHEMA,
        MEDIA_POOL_NAME_SCHEMA,
        MEDIA_SET_NAMING_TEMPLATE_SCHEMA,
        MEDIA_SET_ALLOCATION_POLICY_SCHEMA,
        MEDIA_RETENTION_POLICY_SCHEMA,
        TAPE_ENCRYPTION_KEY_FINGERPRINT_SCHEMA,
        MediaPoolConfig,
    },
    config::{
        self,
        drive::{
            check_drive_exists,
        },
    },
};

#[api(
    input: {
        properties: {
            name: {
                schema: MEDIA_POOL_NAME_SCHEMA,
            },
            drive: {
                schema: DRIVE_NAME_SCHEMA,
            },
            allocation: {
                schema: MEDIA_SET_ALLOCATION_POLICY_SCHEMA,
                optional: true,
            },
            retention: {
                schema: MEDIA_RETENTION_POLICY_SCHEMA,
                optional: true,
            },
            template: {
                schema: MEDIA_SET_NAMING_TEMPLATE_SCHEMA,
                optional: true,
            },
            encrypt: {
                schema: TAPE_ENCRYPTION_KEY_FINGERPRINT_SCHEMA,
                optional: true,
            },
        },
    },
)]
/// Create a new media pool
pub fn create_pool(
    name: String,
    drive: String,
    allocation: Option<String>,
    retention: Option<String>,
    template: Option<String>,
    encrypt: Option<String>,
) -> Result<(), Error> {

    let _lock = config::media_pool::lock()?;

    let (mut config, _digest) = config::media_pool::config()?;

    if config.sections.get(&name).is_some() {
        bail!("Media pool '{}' already exists", name);
    }

    let (drive_config, _) = config::drive::config()?;
    check_drive_exists(&drive_config, &drive)?;

    let item = MediaPoolConfig {
        name: name.clone(),
        drive,
        allocation,
        retention,
        template,
        encrypt,
    };

    config.set_data(&name, "pool", &item)?;

    config::media_pool::save_config(&config)?;

    Ok(())
}

#[api(
    returns: {
        description: "The list of configured media pools (with config digest).",
        type: Array,
        items: {
            type: MediaPoolConfig,
        },
    },
)]
/// List media pools
pub fn list_pools(
    mut rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<MediaPoolConfig>, Error> {

    let (config, digest) = config::media_pool::config()?;

    let list = config.convert_to_typed_array("pool")?;

    rpcenv["digest"] = proxmox::tools::digest_to_hex(&digest).into();

    Ok(list)
}

#[api(
    input: {
        properties: {
            name: {
                schema: MEDIA_POOL_NAME_SCHEMA,
            },
        },
    },
    returns: {
        type: MediaPoolConfig,
    },
)]
/// Get media pool configuration
pub fn get_config(name: String) -> Result<MediaPoolConfig, Error> {

    let (config, _digest) = config::media_pool::config()?;

    let data: MediaPoolConfig = config.lookup("pool", &name)?;

    Ok(data)
}

#[api()]
#[derive(Serialize, Deserialize)]
#[allow(non_camel_case_types)]
/// Deletable property name
pub enum DeletableProperty {
    /// Delete media set allocation policy.
    allocation,
    /// Delete pool retention policy
    retention,
    /// Delete media set naming template
    template,
    /// Delete encryption fingerprint
    encrypt,
}

#[api(
    input: {
        properties: {
            name: {
                schema: MEDIA_POOL_NAME_SCHEMA,
            },
            drive: {
                schema: DRIVE_NAME_SCHEMA,
                optional: true,
            },
            allocation: {
                schema: MEDIA_SET_ALLOCATION_POLICY_SCHEMA,
                optional: true,
            },
            retention: {
                schema: MEDIA_RETENTION_POLICY_SCHEMA,
                optional: true,
            },
            template: {
                schema: MEDIA_SET_NAMING_TEMPLATE_SCHEMA,
                optional: true,
            },
            encrypt: {
                schema: TAPE_ENCRYPTION_KEY_FINGERPRINT_SCHEMA,
                optional: true,
            },
            delete: {
                description: "List of properties to delete.",
                type: Array,
                optional: true,
                items: {
                    type: DeletableProperty,
                }
            },
       },
    },
)]
/// Update media pool settings
pub fn update_pool(
    name: String,
    drive: Option<String>,
    allocation: Option<String>,
    retention: Option<String>,
    template: Option<String>,
    encrypt: Option<String>,
    delete: Option<Vec<DeletableProperty>>,
) -> Result<(), Error> {

    let _lock = config::media_pool::lock()?;

    let (mut config, _digest) = config::media_pool::config()?;

    let mut data: MediaPoolConfig = config.lookup("pool", &name)?;

    if let Some(delete) = delete {
        for delete_prop in delete {
            match delete_prop {
                DeletableProperty::allocation => { data.allocation = None; },
                DeletableProperty::retention => { data.retention = None; },
                DeletableProperty::template => { data.template = None; },
                DeletableProperty::encrypt => { data.encrypt = None; },
            }
        }
    }

    if let Some(drive) = drive { data.drive = drive; }
    if allocation.is_some() { data.allocation = allocation; }
    if retention.is_some() { data.retention = retention; }
    if template.is_some() { data.template = template; }
    if encrypt.is_some() { data.encrypt = encrypt; }

    config.set_data(&name, "pool", &data)?;

    config::media_pool::save_config(&config)?;

    Ok(())
}

#[api(
    input: {
        properties: {
            name: {
                schema: MEDIA_POOL_NAME_SCHEMA,
            },
        },
    },
)]
/// Delete a media pool configuration
pub fn delete_pool(name: String) -> Result<(), Error> {

    let _lock = config::media_pool::lock()?;

    let (mut config, _digest) = config::media_pool::config()?;

    match config.sections.get(&name) {
        Some(_) => { config.sections.remove(&name); },
        None => bail!("delete pool '{}' failed - no such pool", name),
    }

    config::media_pool::save_config(&config)?;

    Ok(())
}

const ITEM_ROUTER: Router = Router::new()
    .get(&API_METHOD_GET_CONFIG)
    .put(&API_METHOD_UPDATE_POOL)
    .delete(&API_METHOD_DELETE_POOL);


pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_LIST_POOLS)
    .post(&API_METHOD_CREATE_POOL)
    .match_all("name", &ITEM_ROUTER);
