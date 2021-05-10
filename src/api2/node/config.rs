use anyhow::Error;

use proxmox::api::schema::Updatable;
use proxmox::api::{api, Permission, Router, RpcEnvironment};

use crate::api2::types::NODE_SCHEMA;
use crate::config::acl::{PRIV_SYS_AUDIT, PRIV_SYS_MODIFY};
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
pub fn get_node_config(mut rpcenv: &mut dyn RpcEnvironment) -> Result<NodeConfig, Error> {
    let (config, digest) = crate::config::node::config()?;
    rpcenv["digest"] = proxmox::tools::digest_to_hex(&digest).into();
    Ok(config)
}

#[api(
    input: {
        properties: {
            node: { schema: NODE_SCHEMA },
            digest: {
                description: "Digest to protect against concurrent updates",
                optional: true,
            },
            updater: {
                type: NodeConfigUpdater,
                flatten: true,
            },
            delete: {
                description: "Options to remove from the configuration",
                optional: true,
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
    updater: NodeConfigUpdater,
    delete: Option<String>,
    digest: Option<String>,
) -> Result<(), Error> {
    let _lock = crate::config::node::lock()?;
    let (mut config, expected_digest) = crate::config::node::config()?;
    if let Some(digest) = digest {
        // FIXME: GUI doesn't handle our non-inlined digest part here properly...
        if !digest.is_empty() {
            let digest = proxmox::tools::hex_to_digest(&digest)?;
            crate::tools::detect_modified_configuration_file(&digest, &expected_digest)?;
        }
    }

    let delete: Vec<&str> = delete
        .as_deref()
        .unwrap_or("")
        .split(&[' ', ',', ';', '\0'][..])
        .collect();

    config.update_from(updater, &delete)?;

    crate::config::node::save_config(&config)
}
