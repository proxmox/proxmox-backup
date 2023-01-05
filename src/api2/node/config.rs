use ::serde::{Deserialize, Serialize};
use anyhow::Error;
use hex::FromHex;

use proxmox_router::{Permission, Router, RpcEnvironment};
use proxmox_schema::api;

use pbs_api_types::{NODE_SCHEMA, PRIV_SYS_AUDIT, PRIV_SYS_MODIFY};

use crate::api2::node::apt::update_apt_proxy_config;
use crate::config::node::{NodeConfig, NodeConfigUpdater};

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_GET_NODE_CONFIG)
    .put(&API_METHOD_UPDATE_NODE_CONFIG);

#[api(
    input: {
        properties: {
            node: { schema: NODE_SCHEMA },
        },
    },
    access: {
        permission: &Permission::Privilege(&["system"], PRIV_SYS_AUDIT, false),
    },
    returns: {
        type: NodeConfig,
    },
)]
/// Get the node configuration
pub fn get_node_config(rpcenv: &mut dyn RpcEnvironment) -> Result<NodeConfig, Error> {
    let (config, digest) = crate::config::node::config()?;
    rpcenv["digest"] = hex::encode(digest).into();
    Ok(config)
}

#[api()]
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
/// Deletable property name
pub enum DeletableProperty {
    /// Delete the acme property.
    Acme,
    /// Delete the acmedomain0 property.
    Acmedomain0,
    /// Delete the acmedomain1 property.
    Acmedomain1,
    /// Delete the acmedomain2 property.
    Acmedomain2,
    /// Delete the acmedomain3 property.
    Acmedomain3,
    /// Delete the acmedomain4 property.
    Acmedomain4,
    /// Delete the http-proxy property.
    HttpProxy,
    /// Delete the email-from property.
    EmailFrom,
    /// Delete the ciphers-tls-1.3 property.
    #[serde(rename = "ciphers-tls-1.3")]
    CiphersTls1_3,
    /// Delete the ciphers-tls-1.2 property.
    #[serde(rename = "ciphers-tls-1.2")]
    CiphersTls1_2,
    /// Delete the default-lang property.
    DefaultLang,
    /// Delete any description
    Description,
    /// Delete the task-log-max-days property
    TaskLogMaxDays,
}

#[api(
    input: {
        properties: {
            node: { schema: NODE_SCHEMA },
            digest: {
                description: "Digest to protect against concurrent updates",
                optional: true,
            },
            update: {
                type: NodeConfigUpdater,
                flatten: true,
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
        permission: &Permission::Privilege(&["system"], PRIV_SYS_MODIFY, false),
    },
    protected: true,
)]
/// Update the node configuration
pub fn update_node_config(
    // node: String, // not used
    update: NodeConfigUpdater,
    delete: Option<Vec<DeletableProperty>>,
    digest: Option<String>,
) -> Result<(), Error> {
    let _lock = crate::config::node::lock()?;
    let (mut config, expected_digest) = crate::config::node::config()?;
    if let Some(digest) = digest {
        // FIXME: GUI doesn't handle our non-inlined digest part here properly...
        if !digest.is_empty() {
            let digest = <[u8; 32]>::from_hex(&digest)?;
            crate::tools::detect_modified_configuration_file(&digest, &expected_digest)?;
        }
    }

    if let Some(delete) = delete {
        for delete_prop in delete {
            match delete_prop {
                DeletableProperty::Acme => {
                    config.acme = None;
                }
                DeletableProperty::Acmedomain0 => {
                    config.acmedomain0 = None;
                }
                DeletableProperty::Acmedomain1 => {
                    config.acmedomain1 = None;
                }
                DeletableProperty::Acmedomain2 => {
                    config.acmedomain2 = None;
                }
                DeletableProperty::Acmedomain3 => {
                    config.acmedomain3 = None;
                }
                DeletableProperty::Acmedomain4 => {
                    config.acmedomain4 = None;
                }
                DeletableProperty::HttpProxy => {
                    config.http_proxy = None;
                }
                DeletableProperty::EmailFrom => {
                    config.email_from = None;
                }
                DeletableProperty::CiphersTls1_3 => {
                    config.ciphers_tls_1_3 = None;
                }
                DeletableProperty::CiphersTls1_2 => {
                    config.ciphers_tls_1_2 = None;
                }
                DeletableProperty::DefaultLang => {
                    config.default_lang = None;
                }
                DeletableProperty::Description => {
                    config.description = None;
                }
                DeletableProperty::TaskLogMaxDays => {
                    config.task_log_max_days = None;
                }
            }
        }
    }

    if update.acme.is_some() {
        config.acme = update.acme;
    }
    if update.acmedomain0.is_some() {
        config.acmedomain0 = update.acmedomain0;
    }
    if update.acmedomain1.is_some() {
        config.acmedomain1 = update.acmedomain1;
    }
    if update.acmedomain2.is_some() {
        config.acmedomain2 = update.acmedomain2;
    }
    if update.acmedomain3.is_some() {
        config.acmedomain3 = update.acmedomain3;
    }
    if update.acmedomain4.is_some() {
        config.acmedomain4 = update.acmedomain4;
    }
    if update.http_proxy.is_some() {
        config.http_proxy = update.http_proxy;
    }
    if update.email_from.is_some() {
        config.email_from = update.email_from;
    }
    if update.ciphers_tls_1_3.is_some() {
        config.ciphers_tls_1_3 = update.ciphers_tls_1_3;
    }
    if update.ciphers_tls_1_2.is_some() {
        config.ciphers_tls_1_2 = update.ciphers_tls_1_2;
    }
    if update.default_lang.is_some() {
        config.default_lang = update.default_lang;
    }
    if update.description.is_some() {
        config.description = update.description;
    }
    if update.task_log_max_days.is_some() {
        config.task_log_max_days = update.task_log_max_days;
    }

    crate::config::node::save_config(&config)?;

    update_apt_proxy_config(config.http_proxy().as_ref())?;

    Ok(())
}
