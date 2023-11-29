use anyhow::{bail, format_err, Error};
use hex::FromHex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use proxmox_metrics::test_influxdb_udp;
use proxmox_router::{Permission, Router, RpcEnvironment};
use proxmox_schema::api;

use pbs_api_types::{
    InfluxDbUdp, InfluxDbUdpUpdater, METRIC_SERVER_ID_SCHEMA, PRIV_SYS_AUDIT, PRIV_SYS_MODIFY,
    PROXMOX_CONFIG_DIGEST_SCHEMA,
};

use pbs_config::metrics;

async fn test_server(address: &str) -> Result<(), Error> {
    test_influxdb_udp(address)
        .await
        .map_err(|err| format_err!("cannot connect to {}: {}", address, err))
}

#[api(
    input: {
        properties: {},
    },
    returns: {
        description: "List of configured InfluxDB udp metric servers.",
        type: Array,
        items: { type: InfluxDbUdp },
    },
    access: {
        permission: &Permission::Privilege(&[], PRIV_SYS_AUDIT, false),
    },
)]
/// List configured InfluxDB udp metric servers.
pub fn list_influxdb_udp_servers(
    _param: Value,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<InfluxDbUdp>, Error> {
    let (config, digest) = metrics::config()?;

    let list = config.convert_to_typed_array("influxdb-udp")?;

    rpcenv["digest"] = hex::encode(digest).into();

    Ok(list)
}

#[api(
    protected: true,
    input: {
        properties: {
            config: {
                type: InfluxDbUdp,
                flatten: true,
            },
        },
    },
    access: {
        permission: &Permission::Privilege(&[], PRIV_SYS_MODIFY, false),
    },
)]
/// Create a new InfluxDB udp server configuration
pub async fn create_influxdb_udp_server(config: InfluxDbUdp) -> Result<(), Error> {
    let _lock = metrics::lock_config()?;

    let (mut metrics, _digest) = metrics::config()?;

    if metrics.sections.get(&config.name).is_some() {
        bail!("metric server '{}' already exists.", config.name);
    }

    if config.enable {
        test_server(&config.host).await?;
    }

    metrics.set_data(&config.name, "influxdb-udp", &config)?;

    metrics::save_config(&metrics)?;

    Ok(())
}

#[api(
    protected: true,
    input: {
        properties: {
            name: {
                schema: METRIC_SERVER_ID_SCHEMA,
            },
            digest: {
                optional: true,
                schema: PROXMOX_CONFIG_DIGEST_SCHEMA,
            },
        },
    },
    access: {
        permission: &Permission::Privilege(&[], PRIV_SYS_MODIFY, false),
    },
)]
/// Remove a InfluxDB udp server configuration
pub fn delete_influxdb_udp_server(
    name: String,
    digest: Option<String>,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {
    let _lock = metrics::lock_config()?;

    let (mut metrics, expected_digest) = metrics::config()?;

    if let Some(ref digest) = digest {
        let digest = <[u8; 32]>::from_hex(digest)?;
        crate::tools::detect_modified_configuration_file(&digest, &expected_digest)?;
    }

    if metrics.sections.remove(&name).is_none() {
        bail!("name '{}' does not exist.", name);
    }

    metrics::save_config(&metrics)?;

    Ok(())
}

#[api(
    input: {
        properties: {
            name: {
                schema: METRIC_SERVER_ID_SCHEMA,
            },
        },
    },
    returns:  { type: InfluxDbUdp },
    access: {
        permission: &Permission::Privilege(&[], PRIV_SYS_AUDIT, false),
    },
)]
/// Read the InfluxDB udp server configuration
pub fn read_influxdb_udp_server(
    name: String,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<InfluxDbUdp, Error> {
    let (metrics, digest) = metrics::config()?;

    let config = metrics.lookup("influxdb-udp", &name)?;

    rpcenv["digest"] = hex::encode(digest).into();

    Ok(config)
}

#[api()]
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
/// Deletable property name
pub enum DeletableProperty {
    /// Delete the enable property.
    Enable,
    /// Delete the mtu property.
    Mtu,
    /// Delete the comment property.
    Comment,
}

#[api(
    protected: true,
    input: {
        properties: {
            name: {
                schema: METRIC_SERVER_ID_SCHEMA,
            },
            update: {
                type: InfluxDbUdpUpdater,
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
            digest: {
                optional: true,
                schema: PROXMOX_CONFIG_DIGEST_SCHEMA,
            },
        },
    },
    access: {
        permission: &Permission::Privilege(&[], PRIV_SYS_MODIFY, false),
    },
)]
/// Update an InfluxDB udp server configuration
pub async fn update_influxdb_udp_server(
    name: String,
    update: InfluxDbUdpUpdater,
    delete: Option<Vec<DeletableProperty>>,
    digest: Option<String>,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {
    let _lock = metrics::lock_config()?;

    let (mut metrics, expected_digest) = metrics::config()?;

    if let Some(ref digest) = digest {
        let digest = <[u8; 32]>::from_hex(digest)?;
        crate::tools::detect_modified_configuration_file(&digest, &expected_digest)?;
    }

    let mut config: InfluxDbUdp = metrics.lookup("influxdb-udp", &name)?;

    if let Some(delete) = delete {
        for delete_prop in delete {
            match delete_prop {
                DeletableProperty::Enable => {
                    config.enable = true;
                }
                DeletableProperty::Mtu => {
                    config.mtu = None;
                }
                DeletableProperty::Comment => {
                    config.comment = None;
                }
            }
        }
    }

    if let Some(comment) = update.comment {
        let comment = comment.trim().to_string();
        if comment.is_empty() {
            config.comment = None;
        } else {
            config.comment = Some(comment);
        }
    }

    if let Some(host) = update.host {
        config.host = host;
    }

    if let Some(enable) = update.enable {
        config.enable = enable;
    }

    if update.mtu.is_some() {
        config.mtu = update.mtu;
    }

    metrics.set_data(&name, "influxdb-udp", &config)?;

    if config.enable {
        test_server(&config.host).await?;
    }

    metrics::save_config(&metrics)?;

    Ok(())
}

const ITEM_ROUTER: Router = Router::new()
    .get(&API_METHOD_READ_INFLUXDB_UDP_SERVER)
    .put(&API_METHOD_UPDATE_INFLUXDB_UDP_SERVER)
    .delete(&API_METHOD_DELETE_INFLUXDB_UDP_SERVER);

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_LIST_INFLUXDB_UDP_SERVERS)
    .post(&API_METHOD_CREATE_INFLUXDB_UDP_SERVER)
    .match_all("name", &ITEM_ROUTER);
