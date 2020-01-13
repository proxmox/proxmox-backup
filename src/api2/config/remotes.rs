use failure::*;
use serde_json::Value;

use proxmox::api::{api, ApiMethod, Router, RpcEnvironment};

use crate::api2::types::*;
use crate::config::remotes;

#[api(
    input: {
        properties: {},
    },
    returns: {
        description: "The list of configured remotes.",
        type: Array,
        items: {
            type: remotes::Remote,
        },
    },
)]
/// List all remotes
pub fn list_remotes(
    _param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let config = remotes::config()?;

    Ok(config.convert_to_array("name"))
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
                schema: remotes::COMMENT_SCHEMA,
            },
            host: {
                schema: remotes::REMOTE_HOST_SCHEMA,
            },
            userid: {
                schema: remotes::REMOTE_USERID_SCHEMA,
            },
            password: {
                schema: remotes::REMOTE_PASSWORD_SCHEMA,
            },
        },
    },
)]
/// Create new remote.
pub fn create_remote(name: String, param: Value) -> Result<(), Error> {

    // fixme: locking ?

    let remote: remotes::Remote = serde_json::from_value(param.clone())?;

    let mut config = remotes::config()?;

    if let Some(_) = config.sections.get(&name) {
        bail!("remote '{}' already exists.", name);
    }

    config.set_data(&name, "remote", &remote)?;

    remotes::save_config(&config)?;

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

    let mut config = remotes::config()?;

    match config.sections.get(&name) {
        Some(_) => { config.sections.remove(&name); },
        None => bail!("remote '{}' does not exist.", name),
    }

    Ok(())
}

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_LIST_REMOTES)
    .post(&API_METHOD_CREATE_REMOTE)
    .delete(&API_METHOD_DELETE_REMOTE);
