use anyhow::Error;
use serde_json::Value;

use proxmox_notify::endpoints::smtp::{
    DeleteableSmtpProperty, SmtpConfig, SmtpConfigUpdater, SmtpPrivateConfig,
    SmtpPrivateConfigUpdater,
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
        description: "List of smtp endpoints.",
        type: Array,
        items: { type: SmtpConfig },
    },
    access: {
        permission: &Permission::Privilege(&["system", "notifications"], PRIV_SYS_AUDIT, false),
    },
)]
/// List all smtp endpoints.
pub fn list_endpoints(
    _param: Value,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<SmtpConfig>, Error> {
    let config = pbs_config::notifications::config()?;

    let endpoints = proxmox_notify::api::smtp::get_endpoints(&config)?;

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
    returns: { type: SmtpConfig },
    access: {
        permission: &Permission::Privilege(&["system", "notifications"], PRIV_SYS_AUDIT, false),
    },
)]
/// Get a smtp endpoint.
pub fn get_endpoint(name: String, rpcenv: &mut dyn RpcEnvironment) -> Result<SmtpConfig, Error> {
    let config = pbs_config::notifications::config()?;
    let endpoint = proxmox_notify::api::smtp::get_endpoint(&config, &name)?;

    rpcenv["digest"] = hex::encode(config.digest()).into();

    Ok(endpoint)
}

#[api(
    protected: true,
    input: {
        properties: {
            endpoint: {
                type: SmtpConfig,
                flatten: true,
            },
            password: {
                optional: true,
                description: "SMTP authentication password"
            }
        },
    },
    access: {
        permission: &Permission::Privilege(&["system", "notifications"], PRIV_SYS_MODIFY, false),
    },
)]
/// Add a new smtp endpoint.
pub fn add_endpoint(
    endpoint: SmtpConfig,
    password: Option<String>,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {
    let _lock = pbs_config::notifications::lock_config()?;
    let mut config = pbs_config::notifications::config()?;
    let private_endpoint_config = SmtpPrivateConfig {
        name: endpoint.name.clone(),
        password,
    };

    proxmox_notify::api::smtp::add_endpoint(&mut config, endpoint, private_endpoint_config)?;

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
                type: SmtpConfigUpdater,
                flatten: true,
            },
            password: {
                description: "SMTP authentication password",
                optional: true,
            },
            delete: {
                description: "List of properties to delete.",
                type: Array,
                optional: true,
                items: {
                    type: DeleteableSmtpProperty,
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
/// Update smtp endpoint.
pub fn update_endpoint(
    name: String,
    updater: SmtpConfigUpdater,
    password: Option<String>,
    delete: Option<Vec<DeleteableSmtpProperty>>,
    digest: Option<String>,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {
    let _lock = pbs_config::notifications::lock_config()?;
    let mut config = pbs_config::notifications::config()?;
    let digest = digest.map(hex::decode).transpose()?;

    proxmox_notify::api::smtp::update_endpoint(
        &mut config,
        &name,
        updater,
        SmtpPrivateConfigUpdater { password },
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
/// Delete smtp endpoint.
pub fn delete_endpoint(name: String, _rpcenv: &mut dyn RpcEnvironment) -> Result<(), Error> {
    let _lock = pbs_config::notifications::lock_config()?;
    let mut config = pbs_config::notifications::config()?;
    proxmox_notify::api::smtp::delete_endpoint(&mut config, &name)?;

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
