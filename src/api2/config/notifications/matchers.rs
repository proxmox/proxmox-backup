use anyhow::Error;
use serde_json::Value;

use proxmox_notify::matcher::{DeleteableMatcherProperty, MatcherConfig, MatcherConfigUpdater};
use proxmox_notify::schema::ENTITY_NAME_SCHEMA;
use proxmox_router::{Permission, Router, RpcEnvironment};
use proxmox_schema::api;

use pbs_api_types::{PRIV_SYS_AUDIT, PRIV_SYS_MODIFY, PROXMOX_CONFIG_DIGEST_SCHEMA};

#[api(
    protected: true,
    input: {
        properties: {},
    },
    returns: {
        description: "List of matchers.",
        type: Array,
        items: { type: MatcherConfig },
    },
    access: {
        permission: &Permission::Privilege(&["system", "notifications"], PRIV_SYS_AUDIT, false),
    },
)]
/// List all notification matchers.
pub fn list_matchers(
    _param: Value,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<MatcherConfig>, Error> {
    let config = pbs_config::notifications::config()?;

    let matchers = proxmox_notify::api::matcher::get_matchers(&config)?;

    Ok(matchers)
}

#[api(
    protected: true,
    input: {
        properties: {
            name: {
                schema: ENTITY_NAME_SCHEMA,
            }
        },
    },
    returns: { type: MatcherConfig },
    access: {
        permission: &Permission::Privilege(&["system", "notifications"], PRIV_SYS_AUDIT, false),
    },
)]
/// Get a notification matcher.
pub fn get_matcher(name: String, rpcenv: &mut dyn RpcEnvironment) -> Result<MatcherConfig, Error> {
    let config = pbs_config::notifications::config()?;
    let matcher = proxmox_notify::api::matcher::get_matcher(&config, &name)?;

    rpcenv["digest"] = hex::encode(config.digest()).into();

    Ok(matcher)
}

#[api(
    protected: true,
    input: {
        properties: {
            matcher: {
                type: MatcherConfig,
                flatten: true,
            }
        },
    },
    access: {
        permission: &Permission::Privilege(&["system", "notifications"], PRIV_SYS_MODIFY, false),
    },
)]
/// Add a new notification matcher.
pub fn add_matcher(matcher: MatcherConfig, _rpcenv: &mut dyn RpcEnvironment) -> Result<(), Error> {
    let _lock = pbs_config::notifications::lock_config()?;
    let mut config = pbs_config::notifications::config()?;

    proxmox_notify::api::matcher::add_matcher(&mut config, matcher)?;

    pbs_config::notifications::save_config(config)?;
    Ok(())
}

#[api(
    protected: true,
    input: {
        properties: {
            name: {
                schema: ENTITY_NAME_SCHEMA,
            },
            updater: {
                type: MatcherConfigUpdater,
                flatten: true,
            },
            delete: {
                description: "List of properties to delete.",
                type: Array,
                optional: true,
                items: {
                    type: DeleteableMatcherProperty,
                }
            },
            digest: {
                optional: true,
                schema: PROXMOX_CONFIG_DIGEST_SCHEMA,
            },
        },
    },
    access: {
        permission: &Permission::Privilege(&["system", "notifications"], PRIV_SYS_MODIFY, false),
    },
)]
/// Update notification matcher.
pub fn update_matcher(
    name: String,
    updater: MatcherConfigUpdater,
    delete: Option<Vec<DeleteableMatcherProperty>>,
    digest: Option<String>,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {
    let _lock = pbs_config::notifications::lock_config()?;
    let mut config = pbs_config::notifications::config()?;
    let digest = digest.map(hex::decode).transpose()?;

    proxmox_notify::api::matcher::update_matcher(
        &mut config,
        &name,
        updater,
        delete.as_deref(),
        digest.as_deref(),
    )?;

    pbs_config::notifications::save_config(config)?;
    Ok(())
}

#[api(
    protected: true,
    input: {
        properties: {
            name: {
                schema: ENTITY_NAME_SCHEMA,
            }
        },
    },
    access: {
        permission: &Permission::Privilege(&["system", "notifications"], PRIV_SYS_MODIFY, false),
    },
)]
/// Delete notification matcher.
pub fn delete_matcher(name: String, _rpcenv: &mut dyn RpcEnvironment) -> Result<(), Error> {
    let _lock = pbs_config::notifications::lock_config()?;
    let mut config = pbs_config::notifications::config()?;
    proxmox_notify::api::matcher::delete_matcher(&mut config, &name)?;

    pbs_config::notifications::save_config(config)?;
    Ok(())
}

const ITEM_ROUTER: Router = Router::new()
    .get(&API_METHOD_GET_MATCHER)
    .put(&API_METHOD_UPDATE_MATCHER)
    .delete(&API_METHOD_DELETE_MATCHER);

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_LIST_MATCHERS)
    .post(&API_METHOD_ADD_MATCHER)
    .match_all("name", &ITEM_ROUTER);
