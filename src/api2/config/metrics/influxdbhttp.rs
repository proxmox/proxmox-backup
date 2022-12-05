use anyhow::{bail, format_err, Error};
use hex::FromHex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use proxmox_metrics::test_influxdb_http;
use proxmox_router::{Permission, Router, RpcEnvironment};
use proxmox_schema::api;

use pbs_api_types::{
    InfluxDbHttp, InfluxDbHttpUpdater, METRIC_SERVER_ID_SCHEMA, PRIV_SYS_AUDIT, PRIV_SYS_MODIFY,
    PROXMOX_CONFIG_DIGEST_SCHEMA,
};

use pbs_config::metrics;

async fn test_server(config: &InfluxDbHttp) -> Result<(), Error> {
    if config.enable {
        test_influxdb_http(
            &config.url,
            config.organization.as_deref().unwrap_or("proxmox"),
            config.bucket.as_deref().unwrap_or("proxmox"),
            config.token.as_deref(),
            config.verify_tls.unwrap_or(true),
        )
        .await
        .map_err(|err| format_err!("could not connect to {}: {}", config.url, err))
    } else {
        Ok(())
    }
}

#[api(
    input: {
        properties: {},
    },
    returns: {
        description: "List of configured InfluxDB http metric servers.",
        type: Array,
        items: { type: InfluxDbHttp },
    },
    access: {
        permission: &Permission::Privilege(&[], PRIV_SYS_AUDIT, false),
    },
)]
/// List configured InfluxDB http metric servers.
pub fn list_influxdb_http_servers(
    _param: Value,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<InfluxDbHttp>, Error> {
    let (config, digest) = metrics::config()?;

    let mut list: Vec<InfluxDbHttp> = config.convert_to_typed_array("influxdb-http")?;

    // don't return token via api
    for item in list.iter_mut() {
        item.token = None;
    }

    rpcenv["digest"] = hex::encode(digest).into();

    Ok(list)
}

#[api(
    protected: true,
    input: {
        properties: {
            config: {
                type: InfluxDbHttp,
                flatten: true,
            },
        },
    },
    access: {
        permission: &Permission::Privilege(&[], PRIV_SYS_MODIFY, false),
    },
)]
/// Create a new InfluxDB http server configuration
pub async fn create_influxdb_http_server(config: InfluxDbHttp) -> Result<(), Error> {
    let _lock = metrics::lock_config()?;

    let (mut metrics, _digest) = metrics::config()?;

    if metrics.sections.get(&config.name).is_some() {
        bail!("metric server '{}' already exists.", config.name);
    }

    test_server(&config).await?;

    metrics.set_data(&config.name, "influxdb-http", &config)?;

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
/// Remove a InfluxDB http server configuration
pub fn delete_influxdb_http_server(
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
    returns:  { type: InfluxDbHttp },
    access: {
        permission: &Permission::Privilege(&[], PRIV_SYS_AUDIT, false),
    },
)]
/// Read the InfluxDB http server configuration
pub fn read_influxdb_http_server(
    name: String,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<InfluxDbHttp, Error> {
    let (metrics, digest) = metrics::config()?;

    let mut config: InfluxDbHttp = metrics.lookup("influxdb-http", &name)?;

    config.token = None;

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
    /// Delete the token property.
    Token,
    /// Delete the bucket property.
    Bucket,
    /// Delete the organization property.
    Organization,
    /// Delete the max_body_size property.
    MaxBodySize,
    /// Delete the verify_tls property.
    VerifyTls,
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
                type: InfluxDbHttpUpdater,
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
/// Update an InfluxDB http server configuration
pub async fn update_influxdb_http_server(
    name: String,
    update: InfluxDbHttpUpdater,
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

    let mut config: InfluxDbHttp = metrics.lookup("influxdb-http", &name)?;

    if let Some(delete) = delete {
        for delete_prop in delete {
            match delete_prop {
                DeletableProperty::Enable => {
                    config.enable = true;
                }
                DeletableProperty::Token => {
                    config.token = None;
                }
                DeletableProperty::Bucket => {
                    config.bucket = None;
                }
                DeletableProperty::Organization => {
                    config.organization = None;
                }
                DeletableProperty::MaxBodySize => {
                    config.max_body_size = None;
                }
                DeletableProperty::VerifyTls => {
                    config.verify_tls = None;
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

    if let Some(url) = update.url {
        config.url = url;
    }

    if let Some(enable) = update.enable {
        config.enable = enable;
    }

    if update.token.is_some() {
        config.token = update.token;
    }
    if update.bucket.is_some() {
        config.bucket = update.bucket;
    }
    if update.organization.is_some() {
        config.organization = update.organization;
    }
    if update.max_body_size.is_some() {
        config.max_body_size = update.max_body_size;
    }
    if update.verify_tls.is_some() {
        config.verify_tls = update.verify_tls;
    }

    test_server(&config).await?;

    metrics.set_data(&name, "influxdb-http", &config)?;

    metrics::save_config(&metrics)?;

    Ok(())
}

const ITEM_ROUTER: Router = Router::new()
    .get(&API_METHOD_READ_INFLUXDB_HTTP_SERVER)
    .put(&API_METHOD_UPDATE_INFLUXDB_HTTP_SERVER)
    .delete(&API_METHOD_DELETE_INFLUXDB_HTTP_SERVER);

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_LIST_INFLUXDB_HTTP_SERVERS)
    .post(&API_METHOD_CREATE_INFLUXDB_HTTP_SERVER)
    .match_all("name", &ITEM_ROUTER);
