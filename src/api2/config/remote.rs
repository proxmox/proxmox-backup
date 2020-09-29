use anyhow::{bail, Error};
use serde_json::Value;
use ::serde::{Deserialize, Serialize};
use base64;

use proxmox::api::{api, ApiMethod, Router, RpcEnvironment, Permission};
use proxmox::tools::fs::open_file_locked;

use crate::api2::types::*;
use crate::config::remote;
use crate::config::acl::{PRIV_REMOTE_AUDIT, PRIV_REMOTE_MODIFY};

#[api(
    input: {
        properties: {},
    },
    returns: {
        description: "The list of configured remotes (with config digest).",
        type: Array,
        items: {
            type: remote::Remote,
            description: "Remote configuration (without password).",
        },
    },
    access: {
        permission: &Permission::Privilege(&["remote"], PRIV_REMOTE_AUDIT, false),
    },
)]
/// List all remotes
pub fn list_remotes(
    _param: Value,
    _info: &ApiMethod,
    mut rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<remote::Remote>, Error> {

    let (config, digest) = remote::config()?;

    let mut list: Vec<remote::Remote> = config.convert_to_typed_array("remote")?;

    // don't return password in api
    for remote in &mut list {
        remote.password = "".to_string();
    }

    rpcenv["digest"] = proxmox::tools::digest_to_hex(&digest).into();
    Ok(list)
}

#[api(
    protected: true,
    input: {
        properties: {
            name: {
                schema: REMOTE_ID_SCHEMA,
            },
            comment: {
                optional: true,
                schema: SINGLE_LINE_COMMENT_SCHEMA,
            },
            host: {
                schema: DNS_NAME_OR_IP_SCHEMA,
            },
            port: {
                description: "The (optional) port.",
                type: u16,
                optional: true,
                default: 8007,
            },
            userid: {
                type: Userid,
            },
            password: {
                schema: remote::REMOTE_PASSWORD_SCHEMA,
            },
            fingerprint: {
                optional: true,
                schema: CERT_FINGERPRINT_SHA256_SCHEMA,
            },
        },
    },
    access: {
        permission: &Permission::Privilege(&["remote"], PRIV_REMOTE_MODIFY, false),
    },
)]
/// Create new remote.
pub fn create_remote(password: String, param: Value) -> Result<(), Error> {

    let _lock = open_file_locked(remote::REMOTE_CFG_LOCKFILE, std::time::Duration::new(10, 0), true)?;

    let mut data = param.clone();
    data["password"] = Value::from(base64::encode(password.as_bytes()));
    let remote: remote::Remote = serde_json::from_value(data)?;

    let (mut config, _digest) = remote::config()?;

    if let Some(_) = config.sections.get(&remote.name) {
        bail!("remote '{}' already exists.", remote.name);
    }

    config.set_data(&remote.name, "remote", &remote)?;

    remote::save_config(&config)?;

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
    returns: {
        description: "The remote configuration (with config digest).",
        type: remote::Remote,
    },
    access: {
        permission: &Permission::Privilege(&["remote", "{name}"], PRIV_REMOTE_AUDIT, false),
    }
)]
/// Read remote configuration data.
pub fn read_remote(
    name: String,
    _info: &ApiMethod,
    mut rpcenv: &mut dyn RpcEnvironment,
) -> Result<remote::Remote, Error> {
    let (config, digest) = remote::config()?;
    let mut data: remote::Remote = config.lookup("remote", &name)?;
    data.password = "".to_string(); // do not return password in api
    rpcenv["digest"] = proxmox::tools::digest_to_hex(&digest).into();
    Ok(data)
}

#[api()]
#[derive(Serialize, Deserialize)]
#[allow(non_camel_case_types)]
/// Deletable property name
pub enum DeletableProperty {
    /// Delete the comment property.
    comment,
    /// Delete the fingerprint property.
    fingerprint,
    /// Delete the port property.
    port,
}

#[api(
    protected: true,
    input: {
        properties: {
            name: {
                schema: REMOTE_ID_SCHEMA,
            },
            comment: {
                optional: true,
                schema: SINGLE_LINE_COMMENT_SCHEMA,
            },
            host: {
                optional: true,
                schema: DNS_NAME_OR_IP_SCHEMA,
            },
            port: {
                description: "The (optional) port.",
                type: u16,
                optional: true,
            },
            userid: {
                optional: true,
                type: Userid,
            },
            password: {
                optional: true,
                schema: remote::REMOTE_PASSWORD_SCHEMA,
            },
            fingerprint: {
                optional: true,
                schema: CERT_FINGERPRINT_SHA256_SCHEMA,
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
    comment: Option<String>,
    host: Option<String>,
    port: Option<u16>,
    userid: Option<Userid>,
    password: Option<String>,
    fingerprint: Option<String>,
    delete: Option<Vec<DeletableProperty>>,
    digest: Option<String>,
) -> Result<(), Error> {

    let _lock = open_file_locked(remote::REMOTE_CFG_LOCKFILE, std::time::Duration::new(10, 0), true)?;

    let (mut config, expected_digest) = remote::config()?;

    if let Some(ref digest) = digest {
        let digest = proxmox::tools::hex_to_digest(digest)?;
        crate::tools::detect_modified_configuration_file(&digest, &expected_digest)?;
    }

    let mut data: remote::Remote = config.lookup("remote", &name)?;

    if let Some(delete) = delete {
        for delete_prop in delete {
            match delete_prop {
                DeletableProperty::comment => { data.comment = None; },
                DeletableProperty::fingerprint => { data.fingerprint = None; },
                DeletableProperty::port => { data.port = None; },
            }
        }
    }

    if let Some(comment) = comment {
        let comment = comment.trim().to_string();
        if comment.is_empty() {
            data.comment = None;
        } else {
            data.comment = Some(comment);
        }
    }
    if let Some(host) = host { data.host = host; }
    if port.is_some() { data.port = port; }
    if let Some(userid) = userid { data.userid = userid; }
    if let Some(password) = password { data.password = password; }

    if let Some(fingerprint) = fingerprint { data.fingerprint = Some(fingerprint); }

    config.set_data(&name, "remote", &data)?;

    remote::save_config(&config)?;

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

    let _lock = open_file_locked(remote::REMOTE_CFG_LOCKFILE, std::time::Duration::new(10, 0), true)?;

    let (mut config, expected_digest) = remote::config()?;

    if let Some(ref digest) = digest {
        let digest = proxmox::tools::hex_to_digest(digest)?;
        crate::tools::detect_modified_configuration_file(&digest, &expected_digest)?;
    }

    match config.sections.get(&name) {
        Some(_) => { config.sections.remove(&name); },
        None => bail!("remote '{}' does not exist.", name),
    }

    remote::save_config(&config)?;

    Ok(())
}

const ITEM_ROUTER: Router = Router::new()
    .get(&API_METHOD_READ_REMOTE)
    .put(&API_METHOD_UPDATE_REMOTE)
    .delete(&API_METHOD_DELETE_REMOTE);

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_LIST_REMOTES)
    .post(&API_METHOD_CREATE_REMOTE)
    .match_all("name", &ITEM_ROUTER);
