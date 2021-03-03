use anyhow::{bail, Error};
use ::serde::{Deserialize, Serialize};

use proxmox::{
    api::{
        api,
        Router,
        RpcEnvironment,
        Permission,
    },
};

use crate::{
    api2::types::{
        Authid,
        MEDIA_POOL_NAME_SCHEMA,
        MEDIA_SET_NAMING_TEMPLATE_SCHEMA,
        MEDIA_SET_ALLOCATION_POLICY_SCHEMA,
        MEDIA_RETENTION_POLICY_SCHEMA,
        TAPE_ENCRYPTION_KEY_FINGERPRINT_SCHEMA,
        SINGLE_LINE_COMMENT_SCHEMA,
        MediaPoolConfig,
    },
    config::{
        self,
        cached_user_info::CachedUserInfo,
        acl::{
            PRIV_TAPE_AUDIT,
            PRIV_TAPE_MODIFY,
        },
    },
};

#[api(
    protected: true,
    input: {
        properties: {
            config: {
                type: MediaPoolConfig,
                flatten: true,
            },
        },
    },
    access: {
        permission: &Permission::Privilege(&["tape", "pool"], PRIV_TAPE_MODIFY, false),
    },
)]
/// Create a new media pool
pub fn create_pool(
    config: MediaPoolConfig,
) -> Result<(), Error> {

    let _lock = config::media_pool::lock()?;

    let (mut section_config, _digest) = config::media_pool::config()?;

    if section_config.sections.get(&config.name).is_some() {
        bail!("Media pool '{}' already exists", config.name);
    }

    section_config.set_data(&config.name, "pool", &config)?;

    config::media_pool::save_config(&section_config)?;

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
    access: {
        description: "List configured media pools filtered by Tape.Audit privileges",
        permission: &Permission::Anybody,
    },
)]
/// List media pools
pub fn list_pools(
    mut rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<MediaPoolConfig>, Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let user_info = CachedUserInfo::new()?;

    let (config, digest) = config::media_pool::config()?;

    let list = config.convert_to_typed_array::<MediaPoolConfig>("pool")?;

     let list = list
        .into_iter()
        .filter(|pool| {
            let privs = user_info.lookup_privs(&auth_id, &["tape", "pool", &pool.name]);
            privs & PRIV_TAPE_AUDIT != 0
        })
        .collect();

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
    access: {
        permission: &Permission::Privilege(&["tape", "pool", "{name}"], PRIV_TAPE_AUDIT, false),
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
    /// Delete comment
    comment,
}

#[api(
    protected: true,
    input: {
        properties: {
            name: {
                schema: MEDIA_POOL_NAME_SCHEMA,
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
            comment: {
                optional: true,
                schema: SINGLE_LINE_COMMENT_SCHEMA,
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
    access: {
        permission: &Permission::Privilege(&["tape", "pool", "{name}"], PRIV_TAPE_MODIFY, false),
    },
)]
/// Update media pool settings
pub fn update_pool(
    name: String,
    allocation: Option<String>,
    retention: Option<String>,
    template: Option<String>,
    encrypt: Option<String>,
    comment: Option<String>,
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
                DeletableProperty::comment => { data.comment = None; },
            }
        }
    }

    if allocation.is_some() { data.allocation = allocation; }
    if retention.is_some() { data.retention = retention; }
    if template.is_some() { data.template = template; }
    if encrypt.is_some() { data.encrypt = encrypt; }

    if let Some(comment) = comment {
        let comment = comment.trim();
        if comment.is_empty() {
            data.comment = None;
        } else {
            data.comment = Some(comment.to_string());
        }
    }

    config.set_data(&name, "pool", &data)?;

    config::media_pool::save_config(&config)?;

    Ok(())
}

#[api(
    protected: true,
    input: {
        properties: {
            name: {
                schema: MEDIA_POOL_NAME_SCHEMA,
            },
        },
    },
    access: {
        permission: &Permission::Privilege(&["tape", "pool", "{name}"], PRIV_TAPE_MODIFY, false),
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
