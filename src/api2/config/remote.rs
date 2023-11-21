use ::serde::{Deserialize, Serialize};
use anyhow::{bail, format_err, Error};
use hex::FromHex;
use pbs_api_types::BackupNamespace;
use pbs_api_types::NamespaceListItem;
use proxmox_router::list_subdirs_api_method;
use proxmox_router::SubdirMap;
use proxmox_sortable_macro::sortable;
use serde_json::Value;

use proxmox_router::{http_bail, http_err, ApiMethod, Permission, Router, RpcEnvironment};
use proxmox_schema::{api, param_bail};

use pbs_api_types::{
    Authid, DataStoreListItem, GroupListItem, RateLimitConfig, Remote, RemoteConfig,
    RemoteConfigUpdater, RemoteWithoutPassword, SyncJobConfig, DATASTORE_SCHEMA, PRIV_REMOTE_AUDIT,
    PRIV_REMOTE_MODIFY, PROXMOX_CONFIG_DIGEST_SCHEMA, REMOTE_ID_SCHEMA, REMOTE_PASSWORD_SCHEMA,
};
use pbs_client::{HttpClient, HttpClientOptions};
use pbs_config::sync;

use pbs_config::CachedUserInfo;
use serde_json::json;

#[api(
    input: {
        properties: {},
    },
    returns: {
        description: "The list of configured remotes (with config digest).",
        type: Array,
        items: { type: RemoteWithoutPassword },
    },
    access: {
        description: "List configured remotes filtered by Remote.Audit privileges",
        permission: &Permission::Anybody,
    },
)]
/// List all remotes
pub fn list_remotes(
    _param: Value,
    _info: &ApiMethod,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<RemoteWithoutPassword>, Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let user_info = CachedUserInfo::new()?;

    let (config, digest) = pbs_config::remote::config()?;

    // Note: This removes the password (we do not want to return the password).
    let list: Vec<RemoteWithoutPassword> = config.convert_to_typed_array("remote")?;

    let list = list
        .into_iter()
        .filter(|remote| {
            let privs = user_info.lookup_privs(&auth_id, &["remote", &remote.name]);
            privs & PRIV_REMOTE_AUDIT != 0
        })
        .collect();

    rpcenv["digest"] = hex::encode(digest).into();
    Ok(list)
}

#[api(
    protected: true,
    input: {
        properties: {
            name: {
                schema: REMOTE_ID_SCHEMA,
            },
            config: {
                type: RemoteConfig,
                flatten: true,
            },
            password: {
                // We expect the plain password here (not base64 encoded)
                schema: REMOTE_PASSWORD_SCHEMA,
            },
        },
    },
    access: {
        permission: &Permission::Privilege(&["remote"], PRIV_REMOTE_MODIFY, false),
    },
)]
/// Create new remote.
pub fn create_remote(name: String, config: RemoteConfig, password: String) -> Result<(), Error> {
    let _lock = pbs_config::remote::lock_config()?;

    let (mut section_config, _digest) = pbs_config::remote::config()?;

    if section_config.sections.get(&name).is_some() {
        param_bail!("name", "remote '{}' already exists.", name);
    }

    let remote = Remote {
        name: name.clone(),
        config,
        password,
    };

    section_config.set_data(&name, "remote", &remote)?;

    pbs_config::remote::save_config(&section_config)?;

    Ok(())
}

#[api(
   input: {
        properties: {
            name: {
                schema: REMOTE_ID_SCHEMA,
            },
        },
    },
    returns: { type: RemoteWithoutPassword },
    access: {
        permission: &Permission::Privilege(&["remote", "{name}"], PRIV_REMOTE_AUDIT, false),
    }
)]
/// Read remote configuration data.
pub fn read_remote(
    name: String,
    _info: &ApiMethod,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<RemoteWithoutPassword, Error> {
    let (config, digest) = pbs_config::remote::config()?;
    let data: RemoteWithoutPassword = config.lookup("remote", &name)?;
    rpcenv["digest"] = hex::encode(digest).into();
    Ok(data)
}

#[api()]
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
/// Deletable property name
pub enum DeletableProperty {
    /// Delete the comment property.
    Comment,
    /// Delete the fingerprint property.
    Fingerprint,
    /// Delete the port property.
    Port,
}

#[api(
    protected: true,
    input: {
        properties: {
            name: {
                schema: REMOTE_ID_SCHEMA,
            },
            update: {
                type: RemoteConfigUpdater,
                flatten: true,
            },
            password: {
                // We expect the plain password here (not base64 encoded)
                optional: true,
                schema: REMOTE_PASSWORD_SCHEMA,
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
        permission: &Permission::Privilege(&["remote", "{name}"], PRIV_REMOTE_MODIFY, false),
    },
)]
/// Update remote configuration.
pub fn update_remote(
    name: String,
    update: RemoteConfigUpdater,
    password: Option<String>,
    delete: Option<Vec<DeletableProperty>>,
    digest: Option<String>,
) -> Result<(), Error> {
    let _lock = pbs_config::remote::lock_config()?;

    let (mut config, expected_digest) = pbs_config::remote::config()?;

    if let Some(ref digest) = digest {
        let digest = <[u8; 32]>::from_hex(digest)?;
        crate::tools::detect_modified_configuration_file(&digest, &expected_digest)?;
    }

    let mut data: Remote = config.lookup("remote", &name)?;

    if let Some(delete) = delete {
        for delete_prop in delete {
            match delete_prop {
                DeletableProperty::Comment => {
                    data.config.comment = None;
                }
                DeletableProperty::Fingerprint => {
                    data.config.fingerprint = None;
                }
                DeletableProperty::Port => {
                    data.config.port = None;
                }
            }
        }
    }

    if let Some(comment) = update.comment {
        let comment = comment.trim().to_string();
        if comment.is_empty() {
            data.config.comment = None;
        } else {
            data.config.comment = Some(comment);
        }
    }
    if let Some(host) = update.host {
        data.config.host = host;
    }
    if update.port.is_some() {
        data.config.port = update.port;
    }
    if let Some(auth_id) = update.auth_id {
        data.config.auth_id = auth_id;
    }
    if let Some(password) = password {
        data.password = password;
    }

    if update.fingerprint.is_some() {
        data.config.fingerprint = update.fingerprint;
    }

    config.set_data(&name, "remote", &data)?;

    pbs_config::remote::save_config(&config)?;

    Ok(())
}

#[api(
    protected: true,
    input: {
        properties: {
            name: {
                schema: REMOTE_ID_SCHEMA,
            },
            digest: {
                optional: true,
                schema: PROXMOX_CONFIG_DIGEST_SCHEMA,
            },
        },
    },
    access: {
        permission: &Permission::Privilege(&["remote", "{name}"], PRIV_REMOTE_MODIFY, false),
    },
)]
/// Remove a remote from the configuration file.
pub fn delete_remote(name: String, digest: Option<String>) -> Result<(), Error> {
    let (sync_jobs, _) = sync::config()?;

    let job_list: Vec<SyncJobConfig> = sync_jobs.convert_to_typed_array("sync")?;
    for job in job_list {
        if job.remote.map_or(false, |id| id == name) {
            param_bail!(
                "name",
                "remote '{}' is used by sync job '{}' (datastore '{}')",
                name,
                job.id,
                job.store
            );
        }
    }

    let _lock = pbs_config::remote::lock_config()?;

    let (mut config, expected_digest) = pbs_config::remote::config()?;

    if let Some(ref digest) = digest {
        let digest = <[u8; 32]>::from_hex(digest)?;
        crate::tools::detect_modified_configuration_file(&digest, &expected_digest)?;
    }

    match config.sections.get(&name) {
        Some(_) => {
            config.sections.remove(&name);
        }
        None => http_bail!(NOT_FOUND, "remote '{}' does not exist.", name),
    }

    pbs_config::remote::save_config(&config)?;

    Ok(())
}

/// Helper to get client for remote.cfg entry without login, just config
pub fn remote_client_config(
    remote: &Remote,
    limit: Option<RateLimitConfig>,
) -> Result<HttpClient, Error> {
    let mut options = HttpClientOptions::new_non_interactive(
        remote.password.clone(),
        remote.config.fingerprint.clone(),
    );

    if let Some(limit) = limit {
        options = options.rate_limit(limit);
    }

    let client = HttpClient::new(
        &remote.config.host,
        remote.config.port.unwrap_or(8007),
        &remote.config.auth_id,
        options,
    )?;

    Ok(client)
}

/// Helper to get client for remote.cfg entry
pub async fn remote_client(
    remote: &Remote,
    limit: Option<RateLimitConfig>,
) -> Result<HttpClient, Error> {
    let client = remote_client_config(remote, limit)?;
    let _auth_info = client
        .login() // make sure we can auth
        .await
        .map_err(|err| {
            format_err!(
                "remote connection to '{}' failed - {}",
                remote.config.host,
                err
            )
        })?;

    Ok(client)
}

#[api(
    input: {
        properties: {
            name: {
                schema: REMOTE_ID_SCHEMA,
            },
        },
    },
    access: {
        permission: &Permission::Privilege(&["remote", "{name}"], PRIV_REMOTE_AUDIT, false),
    },
    returns: {
        description: "List the accessible datastores.",
        type: Array,
        items: { type: DataStoreListItem },
    },
)]
/// List datastores of a remote.cfg entry
pub async fn scan_remote_datastores(name: String) -> Result<Vec<DataStoreListItem>, Error> {
    let (remote_config, _digest) = pbs_config::remote::config()?;
    let remote: Remote = remote_config.lookup("remote", &name)?;

    let map_remote_err = |api_err| {
        http_err!(
            INTERNAL_SERVER_ERROR,
            "failed to scan remote '{}' - {}",
            &name,
            api_err
        )
    };

    let client = remote_client(&remote, None).await.map_err(map_remote_err)?;
    let api_res = client
        .get("api2/json/admin/datastore", None)
        .await
        .map_err(map_remote_err)?;
    let parse_res = match api_res.get("data") {
        Some(data) => serde_json::from_value::<Vec<DataStoreListItem>>(data.to_owned()),
        None => bail!("remote {} did not return any datastore list data", &name),
    };

    match parse_res {
        Ok(parsed) => Ok(parsed),
        Err(_) => bail!("Failed to parse remote scan api result."),
    }
}

#[api(
    input: {
        properties: {
            name: {
                schema: REMOTE_ID_SCHEMA,
            },
            store: {
                schema: DATASTORE_SCHEMA,
            },
        },
    },
    access: {
        permission: &Permission::Privilege(&["remote", "{name}"], PRIV_REMOTE_AUDIT, false),
    },
    returns: {
        description: "List the accessible namespaces of a remote datastore.",
        type: Array,
        items: { type: NamespaceListItem },
    },
)]
/// List namespaces of a datastore of a remote.cfg entry
pub async fn scan_remote_namespaces(
    name: String,
    store: String,
) -> Result<Vec<NamespaceListItem>, Error> {
    let (remote_config, _digest) = pbs_config::remote::config()?;
    let remote: Remote = remote_config.lookup("remote", &name)?;

    let map_remote_err = |api_err| {
        http_err!(
            INTERNAL_SERVER_ERROR,
            "failed to scan remote '{}' - {}",
            &name,
            api_err
        )
    };

    let client = remote_client(&remote, None).await.map_err(map_remote_err)?;
    let api_res = client
        .get(
            &format!("api2/json/admin/datastore/{}/namespace", store),
            None,
        )
        .await
        .map_err(map_remote_err)?;
    let parse_res = match api_res.get("data") {
        Some(data) => serde_json::from_value::<Vec<NamespaceListItem>>(data.to_owned()),
        None => bail!("remote {} did not return any datastore list data", &name),
    };

    match parse_res {
        Ok(parsed) => Ok(parsed),
        Err(_) => bail!("Failed to parse remote scan api result."),
    }
}

#[api(
    input: {
        properties: {
            name: {
                schema: REMOTE_ID_SCHEMA,
            },
            store: {
                schema: DATASTORE_SCHEMA,
            },
            namespace: {
                type: BackupNamespace,
                optional: true,
            },
        },
    },
    access: {
        permission: &Permission::Privilege(&["remote", "{name}"], PRIV_REMOTE_AUDIT, false),
    },
    returns: {
        description: "Lists the accessible backup groups in a remote datastore.",
        type: Array,
        items: { type: GroupListItem },
    },
)]
/// List groups of a remote.cfg entry's datastore
pub async fn scan_remote_groups(
    name: String,
    store: String,
    namespace: Option<BackupNamespace>,
) -> Result<Vec<GroupListItem>, Error> {
    let (remote_config, _digest) = pbs_config::remote::config()?;
    let remote: Remote = remote_config.lookup("remote", &name)?;

    let map_remote_err = |api_err| {
        http_err!(
            INTERNAL_SERVER_ERROR,
            "failed to scan remote '{}' - {}",
            &name,
            api_err
        )
    };

    let client = remote_client(&remote, None).await.map_err(map_remote_err)?;

    let args = namespace.map(|ns| json!({ "ns": ns }));

    let api_res = client
        .get(&format!("api2/json/admin/datastore/{}/groups", store), args)
        .await
        .map_err(map_remote_err)?;
    let parse_res = match api_res.get("data") {
        Some(data) => serde_json::from_value::<Vec<GroupListItem>>(data.to_owned()),
        None => bail!("remote {} did not return any group list data", &name),
    };

    match parse_res {
        Ok(parsed) => Ok(parsed),
        Err(_) => bail!("Failed to parse remote scan api result."),
    }
}

#[sortable]
const DATASTORE_SCAN_SUBDIRS: SubdirMap = &sorted!([
    ("groups", &Router::new().get(&API_METHOD_SCAN_REMOTE_GROUPS)),
    (
        "namespaces",
        &Router::new().get(&API_METHOD_SCAN_REMOTE_NAMESPACES),
    ),
]);

const DATASTORE_SCAN_ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(DATASTORE_SCAN_SUBDIRS))
    .subdirs(DATASTORE_SCAN_SUBDIRS);

const SCAN_ROUTER: Router = Router::new()
    .get(&API_METHOD_SCAN_REMOTE_DATASTORES)
    .match_all("store", &DATASTORE_SCAN_ROUTER);

const ITEM_ROUTER: Router = Router::new()
    .get(&API_METHOD_READ_REMOTE)
    .put(&API_METHOD_UPDATE_REMOTE)
    .delete(&API_METHOD_DELETE_REMOTE)
    .subdirs(&[("scan", &SCAN_ROUTER)]);

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_LIST_REMOTES)
    .post(&API_METHOD_CREATE_REMOTE)
    .match_all("name", &ITEM_ROUTER);
