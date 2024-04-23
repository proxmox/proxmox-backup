use anyhow::Error;
use serde_json::Value;

use proxmox_notify::endpoints::sendmail::{
    DeleteableSendmailProperty, SendmailConfig, SendmailConfigUpdater,
};
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
        description: "List of sendmail endpoints.",
        type: Array,
        items: { type: SendmailConfig },
    },
    access: {
        permission: &Permission::Privilege(&["system", "notifications"], PRIV_SYS_AUDIT, false),
    },
)]
/// List all sendmail endpoints.
pub fn list_endpoints(
    _param: Value,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<SendmailConfig>, Error> {
    let config = pbs_config::notifications::config()?;

    let endpoints = proxmox_notify::api::sendmail::get_endpoints(&config)?;

    Ok(endpoints)
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
    returns: { type: SendmailConfig },
    access: {
        permission: &Permission::Privilege(&["system", "notifications"], PRIV_SYS_AUDIT, false),
    },
)]
/// Get a sendmail endpoint.
pub fn get_endpoint(
    name: String,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<SendmailConfig, Error> {
    let config = pbs_config::notifications::config()?;
    let endpoint = proxmox_notify::api::sendmail::get_endpoint(&config, &name)?;

    rpcenv["digest"] = hex::encode(config.digest()).into();

    Ok(endpoint)
}

#[api(
    protected: true,
    input: {
        properties: {
            endpoint: {
                type: SendmailConfig,
                flatten: true,
            }
        },
    },
    access: {
        permission: &Permission::Privilege(&["system", "notifications"], PRIV_SYS_MODIFY, false),
    },
)]
/// Add a new sendmail endpoint.
pub fn add_endpoint(
    endpoint: SendmailConfig,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {
    let _lock = pbs_config::notifications::lock_config()?;
    let mut config = pbs_config::notifications::config()?;

    proxmox_notify::api::sendmail::add_endpoint(&mut config, endpoint)?;

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
                type: SendmailConfigUpdater,
                flatten: true,
            },
            delete: {
                description: "List of properties to delete.",
                type: Array,
                optional: true,
                items: {
                    type: DeleteableSendmailProperty,
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
/// Update sendmail endpoint.
pub fn update_endpoint(
    name: String,
    updater: SendmailConfigUpdater,
    delete: Option<Vec<DeleteableSendmailProperty>>,
    digest: Option<String>,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {
    let _lock = pbs_config::notifications::lock_config()?;
    let mut config = pbs_config::notifications::config()?;
    let digest = digest.map(hex::decode).transpose()?;

    proxmox_notify::api::sendmail::update_endpoint(
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
/// Delete sendmail endpoint.
pub fn delete_endpoint(name: String, _rpcenv: &mut dyn RpcEnvironment) -> Result<(), Error> {
    let _lock = pbs_config::notifications::lock_config()?;
    let mut config = pbs_config::notifications::config()?;
    proxmox_notify::api::sendmail::delete_endpoint(&mut config, &name)?;

    pbs_config::notifications::save_config(config)?;
    Ok(())
}

const ITEM_ROUTER: Router = Router::new()
    .get(&API_METHOD_GET_ENDPOINT)
    .put(&API_METHOD_UPDATE_ENDPOINT)
    .delete(&API_METHOD_DELETE_ENDPOINT);

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_LIST_ENDPOINTS)
    .post(&API_METHOD_ADD_ENDPOINT)
    .match_all("name", &ITEM_ROUTER);
