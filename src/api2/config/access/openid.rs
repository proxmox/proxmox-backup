/// Configure OpenId realms

use anyhow::{bail, Error};
use serde_json::Value;
use ::serde::{Deserialize, Serialize};

use proxmox::api::{api, Permission, Router, RpcEnvironment};

use crate::config::domains::{self, OpenIdRealmConfig};
use crate::config::acl::{PRIV_SYS_AUDIT, PRIV_REALM_ALLOCATE};
use crate::api2::types::*;

#[api(
    input: {
        properties: {},
    },
    returns: {
        description: "List of configured OpenId realms.",
        type: Array,
        items: { type: OpenIdRealmConfig },
    },
    access: {
        permission: &Permission::Privilege(&["access", "domains"], PRIV_REALM_ALLOCATE, false),
    },
)]
/// List configured OpenId realms
pub fn list_openid_realms(
    _param: Value,
    mut rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<OpenIdRealmConfig>, Error> {

    let (config, digest) = domains::config()?;

    let list = config.convert_to_typed_array("openid")?;

    rpcenv["digest"] = proxmox::tools::digest_to_hex(&digest).into();

    Ok(list)
}

#[api(
    protected: true,
    input: {
        properties: {
            config: {
                type: OpenIdRealmConfig,
                flatten: true,
            },
        },
    },
    access: {
        permission: &Permission::Privilege(&["access", "domains"], PRIV_REALM_ALLOCATE, false),
    },
)]
/// Create a new OpenId realm
pub fn create_openid_realm(config: OpenIdRealmConfig) -> Result<(), Error> {

    let _lock = domains::lock_config()?;

    let (mut domains, _digest) = domains::config()?;

    if config.realm == "pbs" ||
        config.realm == "pam" ||
        domains.sections.get(&config.realm).is_some()
    {
        bail!("realm '{}' already exists.", config.realm);
    }

    domains.set_data(&config.realm, "openid", &config)?;

    domains::save_config(&domains)?;

    Ok(())
}

#[api(
    protected: true,
    input: {
        properties: {
            realm: {
                schema: REALM_ID_SCHEMA,
            },
            digest: {
                optional: true,
                schema: PROXMOX_CONFIG_DIGEST_SCHEMA,
            },
        },
    },
    access: {
        permission: &Permission::Privilege(&["access", "domains"], PRIV_REALM_ALLOCATE, false),
    },
)]
/// Remove a OpenID realm configuration
pub fn delete_openid_realm(
    realm: String,
    digest: Option<String>,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {

    let _lock = domains::lock_config()?;

    let (mut domains, expected_digest) = domains::config()?;

    if let Some(ref digest) = digest {
        let digest = proxmox::tools::hex_to_digest(digest)?;
        crate::tools::detect_modified_configuration_file(&digest, &expected_digest)?;
    }

    if domains.sections.remove(&realm).is_none()  {
        bail!("realm '{}' does not exist.", realm);
    }

    domains::save_config(&domains)?;

    Ok(())
}

#[api(
    input: {
        properties: {
            realm: {
                schema: REALM_ID_SCHEMA,
            },
        },
    },
    returns:  { type: OpenIdRealmConfig },
    access: {
        permission: &Permission::Privilege(&["access", "domains"], PRIV_SYS_AUDIT, false),
    },
)]
/// Read the OpenID realm configuration
pub fn read_openid_realm(
    realm: String,
    mut rpcenv: &mut dyn RpcEnvironment,
) -> Result<OpenIdRealmConfig, Error> {

    let (domains, digest) = domains::config()?;

    let config = domains.lookup("openid", &realm)?;

    rpcenv["digest"] = proxmox::tools::digest_to_hex(&digest).into();

    Ok(config)
}

#[api()]
#[derive(Serialize, Deserialize)]
#[serde(rename_all="kebab-case")]
#[allow(non_camel_case_types)]
/// Deletable property name
pub enum DeletableProperty {
    /// Delete the client key.
    client_key,
    /// Delete the comment property.
    comment,
    /// Delete the autocreate property
    autocreate,
}

#[api(
    protected: true,
    input: {
        properties: {
            realm: {
                schema: REALM_ID_SCHEMA,
            },
            "issuer-url": {
                description: "OpenID Issuer Url",
                type: String,
                optional: true,
            },
            "client-id": {
                description: "OpenID Client ID",
                type: String,
                optional: true,
            },
            "client-key": {
                description: "OpenID Client Key",
                type: String,
                optional: true,
            },
            autocreate: {
                description: "Automatically create users if they do not exist.",
                optional: true,
                type: bool,
            },
            comment: {
                schema: SINGLE_LINE_COMMENT_SCHEMA,
                optional: true,
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
    returns:  { type: OpenIdRealmConfig },
    access: {
        permission: &Permission::Privilege(&["access", "domains"], PRIV_REALM_ALLOCATE, false),
    },
)]
/// Update an OpenID realm configuration
pub fn update_openid_realm(
    realm: String,
    issuer_url: Option<String>,
    client_id: Option<String>,
    client_key: Option<String>,
    autocreate: Option<bool>,
    comment: Option<String>,
    delete: Option<Vec<DeletableProperty>>,
    digest: Option<String>,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {

    let _lock = domains::lock_config()?;

    let (mut domains, expected_digest) = domains::config()?;

    if let Some(ref digest) = digest {
        let digest = proxmox::tools::hex_to_digest(digest)?;
        crate::tools::detect_modified_configuration_file(&digest, &expected_digest)?;
    }

    let mut config: OpenIdRealmConfig = domains.lookup("openid", &realm)?;

    if let Some(delete) = delete {
        for delete_prop in delete {
            match delete_prop {
                DeletableProperty::client_key => { config.client_key = None; },
                DeletableProperty::comment => { config.comment = None; },
                DeletableProperty::autocreate => { config.autocreate = None; },
            }
        }
    }

    if let Some(comment) = comment {
        let comment = comment.trim().to_string();
        if comment.is_empty() {
            config.comment = None;
        } else {
            config.comment = Some(comment);
        }
    }

    if let Some(issuer_url) = issuer_url { config.issuer_url = issuer_url; }
    if let Some(client_id) = client_id { config.client_id = client_id; }

    if client_key.is_some() { config.client_key = client_key; }
    if autocreate.is_some() { config.autocreate = autocreate; }

    domains.set_data(&realm, "openid", &config)?;

    domains::save_config(&domains)?;

    Ok(())
}

const ITEM_ROUTER: Router = Router::new()
    .get(&API_METHOD_READ_OPENID_REALM)
    .put(&API_METHOD_UPDATE_OPENID_REALM)
    .delete(&API_METHOD_DELETE_OPENID_REALM);

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_LIST_OPENID_REALMS)
    .post(&API_METHOD_CREATE_OPENID_REALM)
    .match_all("realm", &ITEM_ROUTER);
