use anyhow::{bail, format_err, Error};
use hex::FromHex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use proxmox_ldap::{Config as LdapConfig, Connection};
use proxmox_router::{Permission, Router, RpcEnvironment};
use proxmox_schema::{api, param_bail};

use pbs_api_types::{
    AdRealmConfig, AdRealmConfigUpdater, PRIV_REALM_ALLOCATE, PRIV_SYS_AUDIT,
    PROXMOX_CONFIG_DIGEST_SCHEMA, REALM_ID_SCHEMA,
};

use pbs_config::domains;

use crate::{auth::AdAuthenticator, auth_helpers};

#[api(
    input: {
        properties: {},
    },
    returns: {
        description: "List of configured AD realms.",
        type: Array,
        items: { type: AdRealmConfig },
    },
    access: {
        permission: &Permission::Privilege(&["access", "domains"], PRIV_REALM_ALLOCATE, false),
    },
)]
/// List configured AD realms
pub fn list_ad_realms(
    _param: Value,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<AdRealmConfig>, Error> {
    let (config, digest) = domains::config()?;

    let list = config.convert_to_typed_array("ad")?;

    rpcenv["digest"] = hex::encode(digest).into();

    Ok(list)
}

#[api(
    protected: true,
    input: {
        properties: {
            config: {
                type: AdRealmConfig,
                flatten: true,
            },
            password: {
                description: "AD bind password",
                optional: true,
            }
        },
    },
    access: {
        permission: &Permission::Privilege(&["access", "domains"], PRIV_REALM_ALLOCATE, false),
    },
)]
/// Create a new AD realm
pub async fn create_ad_realm(
    mut config: AdRealmConfig,
    password: Option<String>,
) -> Result<(), Error> {
    let domain_config_lock = domains::lock_config()?;

    let (mut domains, _digest) = domains::config()?;

    if domains::exists(&domains, &config.realm) {
        param_bail!("realm", "realm '{}' already exists.", config.realm);
    }

    let mut ldap_config =
        AdAuthenticator::api_type_to_config_with_password(&config, password.clone())?;

    if config.base_dn.is_none() {
        ldap_config.base_dn = retrieve_default_naming_context(&ldap_config).await?;
        config.base_dn = Some(ldap_config.base_dn.clone());
    }

    let conn = Connection::new(ldap_config);
    conn.check_connection()
        .await
        .map_err(|e| format_err!("{e:#}"))?;

    if let Some(password) = password {
        auth_helpers::store_ldap_bind_password(&config.realm, &password, &domain_config_lock)?;
    }

    domains.set_data(&config.realm, "ad", &config)?;

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
    returns: { type: AdRealmConfig },
    access: {
        permission: &Permission::Privilege(&["access", "domains"], PRIV_SYS_AUDIT, false),
    },
)]
/// Read the AD realm configuration
pub fn read_ad_realm(
    realm: String,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<AdRealmConfig, Error> {
    let (domains, digest) = domains::config()?;

    let config = domains.lookup("ad", &realm)?;

    rpcenv["digest"] = hex::encode(digest).into();

    Ok(config)
}

#[api()]
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
/// Deletable property name
pub enum DeletableProperty {
    /// Fallback AD server address
    Server2,
    /// Port
    Port,
    /// Comment
    Comment,
    /// Verify server certificate
    Verify,
    /// Mode (ldap, ldap+starttls or ldaps),
    Mode,
    /// Bind Domain
    BindDn,
    /// LDAP bind passwort
    Password,
    /// User filter
    Filter,
    /// Default options for user sync
    SyncDefaultsOptions,
    /// user attributes to sync with AD attributes
    SyncAttributes,
    /// User classes
    UserClasses,
}

#[api(
    protected: true,
    input: {
        properties: {
            realm: {
                schema: REALM_ID_SCHEMA,
            },
            update: {
                type: AdRealmConfigUpdater,
                flatten: true,
            },
            password: {
                description: "AD bind password",
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
    returns:  { type: AdRealmConfig },
    access: {
        permission: &Permission::Privilege(&["access", "domains"], PRIV_REALM_ALLOCATE, false),
    },
)]
/// Update an AD realm configuration
pub async fn update_ad_realm(
    realm: String,
    update: AdRealmConfigUpdater,
    password: Option<String>,
    delete: Option<Vec<DeletableProperty>>,
    digest: Option<String>,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {
    let domain_config_lock = domains::lock_config()?;

    let (mut domains, expected_digest) = domains::config()?;

    if let Some(ref digest) = digest {
        let digest = <[u8; 32]>::from_hex(digest)?;
        crate::tools::detect_modified_configuration_file(&digest, &expected_digest)?;
    }

    let mut config: AdRealmConfig = domains.lookup("ad", &realm)?;

    if let Some(delete) = delete {
        for delete_prop in delete {
            match delete_prop {
                DeletableProperty::Server2 => {
                    config.server2 = None;
                }
                DeletableProperty::Comment => {
                    config.comment = None;
                }
                DeletableProperty::Port => {
                    config.port = None;
                }
                DeletableProperty::Verify => {
                    config.verify = None;
                }
                DeletableProperty::Mode => {
                    config.mode = None;
                }
                DeletableProperty::BindDn => {
                    config.bind_dn = None;
                }
                DeletableProperty::Password => {
                    auth_helpers::remove_ldap_bind_password(&realm, &domain_config_lock)?;
                }
                DeletableProperty::Filter => {
                    config.filter = None;
                }
                DeletableProperty::SyncDefaultsOptions => {
                    config.sync_defaults_options = None;
                }
                DeletableProperty::SyncAttributes => {
                    config.sync_attributes = None;
                }
                DeletableProperty::UserClasses => {
                    config.user_classes = None;
                }
            }
        }
    }

    if let Some(server1) = update.server1 {
        config.server1 = server1;
    }

    if let Some(server2) = update.server2 {
        config.server2 = Some(server2);
    }

    if let Some(port) = update.port {
        config.port = Some(port);
    }

    if let Some(base_dn) = update.base_dn {
        config.base_dn = Some(base_dn);
    }

    if let Some(comment) = update.comment {
        let comment = comment.trim().to_string();
        if comment.is_empty() {
            config.comment = None;
        } else {
            config.comment = Some(comment);
        }
    }

    if let Some(mode) = update.mode {
        config.mode = Some(mode);
    }

    if let Some(verify) = update.verify {
        config.verify = Some(verify);
    }

    if let Some(bind_dn) = update.bind_dn {
        config.bind_dn = Some(bind_dn);
    }

    if let Some(filter) = update.filter {
        config.filter = Some(filter);
    }

    if let Some(sync_defaults_options) = update.sync_defaults_options {
        config.sync_defaults_options = Some(sync_defaults_options);
    }

    if let Some(sync_attributes) = update.sync_attributes {
        config.sync_attributes = Some(sync_attributes);
    }

    if let Some(user_classes) = update.user_classes {
        config.user_classes = Some(user_classes);
    }

    let mut ldap_config = if password.is_some() {
        AdAuthenticator::api_type_to_config_with_password(&config, password.clone())?
    } else {
        AdAuthenticator::api_type_to_config(&config)?
    };

    if config.base_dn.is_none() {
        ldap_config.base_dn = retrieve_default_naming_context(&ldap_config).await?;
        config.base_dn = Some(ldap_config.base_dn.clone());
    }

    let conn = Connection::new(ldap_config);
    conn.check_connection()
        .await
        .map_err(|e| format_err!("{e:#}"))?;

    if let Some(password) = password {
        auth_helpers::store_ldap_bind_password(&realm, &password, &domain_config_lock)?;
    }

    domains.set_data(&realm, "ad", &config)?;

    domains::save_config(&domains)?;

    Ok(())
}

async fn retrieve_default_naming_context(ldap_config: &LdapConfig) -> Result<String, Error> {
    let conn = Connection::new(ldap_config.clone());
    match conn.retrieve_root_dse_attr("defaultNamingContext").await {
        Ok(base_dn) if !base_dn.is_empty() => Ok(base_dn[0].clone()),
        Ok(_) => bail!("server did not provide `defaultNamingContext`"),
        Err(err) => bail!("failed to determine base_dn: {err}"),
    }
}

const ITEM_ROUTER: Router = Router::new()
    .get(&API_METHOD_READ_AD_REALM)
    .put(&API_METHOD_UPDATE_AD_REALM)
    .delete(&super::ldap::API_METHOD_DELETE_LDAP_REALM);

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_LIST_AD_REALMS)
    .post(&API_METHOD_CREATE_AD_REALM)
    .match_all("realm", &ITEM_ROUTER);
