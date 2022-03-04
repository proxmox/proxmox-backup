/// Configure OpenId realms

use anyhow::{bail, Error};
use serde_json::Value;
use ::serde::{Deserialize, Serialize};
use hex::FromHex;

use proxmox_router::{Router, RpcEnvironment, Permission};
use proxmox_schema::{api, param_bail};

use pbs_api_types::{
    OpenIdRealmConfig, OpenIdRealmConfigUpdater,
    PROXMOX_CONFIG_DIGEST_SCHEMA, REALM_ID_SCHEMA, PRIV_SYS_AUDIT, PRIV_REALM_ALLOCATE,
};

use pbs_config::domains;

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

    rpcenv["digest"] = hex::encode(&digest).into();

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
        param_bail!("realm", "realm '{}' already exists.", config.realm);
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
        let digest = <[u8; 32]>::from_hex(digest)?;
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

    rpcenv["digest"] = hex::encode(&digest).into();

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
    /// Delete the scopes property
    scopes,
    /// Delete the prompt property
    prompt,
    /// Delete the acr_values property
    acr_values,
}

#[api(
    protected: true,
    input: {
        properties: {
            realm: {
                schema: REALM_ID_SCHEMA,
            },
            update: {
                type: OpenIdRealmConfigUpdater,
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
    returns:  { type: OpenIdRealmConfig },
    access: {
        permission: &Permission::Privilege(&["access", "domains"], PRIV_REALM_ALLOCATE, false),
    },
)]
/// Update an OpenID realm configuration
pub fn update_openid_realm(
    realm: String,
    update: OpenIdRealmConfigUpdater,
    delete: Option<Vec<DeletableProperty>>,
    digest: Option<String>,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {

    let _lock = domains::lock_config()?;

    let (mut domains, expected_digest) = domains::config()?;

    if let Some(ref digest) = digest {
        let digest = <[u8; 32]>::from_hex(digest)?;
        crate::tools::detect_modified_configuration_file(&digest, &expected_digest)?;
    }

    let mut config: OpenIdRealmConfig = domains.lookup("openid", &realm)?;

    if let Some(delete) = delete {
        for delete_prop in delete {
            match delete_prop {
                DeletableProperty::client_key => { config.client_key = None; },
                DeletableProperty::comment => { config.comment = None; },
                DeletableProperty::autocreate => { config.autocreate = None; },
                DeletableProperty::scopes => { config.scopes = None; },
                DeletableProperty::prompt => { config.prompt = None; },
                DeletableProperty::acr_values => { config.acr_values = None; },
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

    if let Some(issuer_url) = update.issuer_url { config.issuer_url = issuer_url; }
    if let Some(client_id) = update.client_id { config.client_id = client_id; }

    if update.client_key.is_some() { config.client_key = update.client_key; }
    if update.autocreate.is_some() { config.autocreate = update.autocreate; }
    if update.scopes.is_some() { config.scopes = update.scopes; }
    if update.prompt.is_some() { config.prompt = update.prompt; }
    if update.acr_values.is_some() { config.acr_values = update.acr_values; }

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
