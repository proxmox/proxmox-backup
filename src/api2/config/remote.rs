use failure::*;
use serde_json::Value;

use proxmox::api::{api, ApiMethod, Router, RpcEnvironment};

use crate::api2::types::*;
use crate::config::remote;

#[api(
    input: {
        properties: {},
    },
    returns: {
        description: "The list of configured remotes (with config digest).",
        type: Array,
        items: {
            type: Object,
            description: "Remote configuration (without password).",
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
                userid: {
                    schema: PROXMOX_USER_ID_SCHEMA,
                },
                fingerprint: {
                    optional: true,
                    schema: CERT_FINGERPRINT_SHA256_SCHEMA,
                },
            },
        },
    },
)]
/// List all remotes
pub fn list_remotes(
    _param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let (config, digest) = remote::config()?;

    let value = config.convert_to_array("name", Some(&digest), &["password"]);
  
    Ok(value.into())
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
            userid: {
                schema: PROXMOX_USER_ID_SCHEMA,
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
)]
/// Create new remote.
pub fn create_remote(name: String, param: Value) -> Result<(), Error> {

    let _lock = crate::tools::open_file_locked(remote::REMOTE_CFG_LOCKFILE, std::time::Duration::new(10, 0))?;

    let remote: remote::Remote = serde_json::from_value(param.clone())?;

    let (mut config, _digest) = remote::config()?;

    if let Some(_) = config.sections.get(&name) {
        bail!("remote '{}' already exists.", name);
    }

    config.set_data(&name, "remote", &remote)?;

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
)]
/// Read remote configuration data.
pub fn read_remote(name: String) -> Result<Value, Error> {
    let (config, digest) = remote::config()?;
    let mut data = config.lookup_json("remote", &name)?;
    data.as_object_mut().unwrap()
        .insert("digest".into(), proxmox::tools::digest_to_hex(&digest).into());
    Ok(data)
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
            userid: {
                optional: true,
               schema: PROXMOX_USER_ID_SCHEMA,
            },
            password: {
                optional: true,
                schema: remote::REMOTE_PASSWORD_SCHEMA,
            },
            fingerprint: {
                optional: true,
                schema: CERT_FINGERPRINT_SHA256_SCHEMA,
            },
            digest: {
                optional: true,
                schema: PROXMOX_CONFIG_DIGEST_SCHEMA,
            },
        },
    },
)]
/// Update remote configuration.
pub fn update_remote(
    name: String,
    comment: Option<String>,
    host: Option<String>,
    userid: Option<String>,
    password: Option<String>,
    fingerprint: Option<String>,
    digest: Option<String>,
) -> Result<(), Error> {

    let _lock = crate::tools::open_file_locked(remote::REMOTE_CFG_LOCKFILE, std::time::Duration::new(10, 0))?;

    let (mut config, expected_digest) = remote::config()?;

    if let Some(ref digest) = digest {
        let digest = proxmox::tools::hex_to_digest(digest)?;
        crate::tools::detect_modified_configuration_file(&digest, &expected_digest)?;
    }

    let mut data: remote::Remote = config.lookup("remote", &name)?;

    if let Some(comment) = comment {
        let comment = comment.trim().to_string();
        if comment.is_empty() {
            data.comment = None;
        } else {
            data.comment = Some(comment);
        }
    }
    if let Some(host) = host { data.host = host; }
    if let Some(userid) = userid { data.userid = userid; }
    if let Some(password) = password { data.password = password; }

    // fixme: howto delete a fingeprint?
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
        },
    },
)]
/// Remove a remote from the configuration file.
pub fn delete_remote(name: String) -> Result<(), Error> {

    // fixme: locking ?
    // fixme: check digest ?

    let (mut config, _digest) = remote::config()?;

    match config.sections.get(&name) {
        Some(_) => { config.sections.remove(&name); },
        None => bail!("remote '{}' does not exist.", name),
    }

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
