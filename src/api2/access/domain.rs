//! List Authentication domains/realms

use anyhow::{bail, Error};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use proxmox::api::{api, Permission, Router, RpcEnvironment};

use crate::api2::types::*;
use crate::config::{
    self,
    acl::{PRIV_REALM_ALLOCATE, PRIV_SYS_AUDIT},
    domains::{OpenIdRealmConfig, OpenIdUserAttribute},
};

#[api]
#[derive(Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
/// type of the realm
pub enum RealmType {
    /// The PAM realm
    Pam,
    /// The PBS realm
    Pbs,
    /// An OpenID Connect realm
    OpenId,
}

#[api(
    properties: {
        realm: {
            schema: REALM_ID_SCHEMA,
        },
        "type": {
            type: RealmType,
        },
        comment: {
            optional: true,
            schema: SINGLE_LINE_COMMENT_SCHEMA,
        },
    },
)]
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
/// Basic Information about a realm
pub struct BasicRealmInfo {
    pub realm: String,
    #[serde(rename = "type")]
    pub ty: RealmType,
    /// True if it is the default realm
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

#[api(
    properties: {
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
            default: false,
        },
        "username-claim": {
            type: OpenIdUserAttribute,
            optional: true,
        },
    },
)]
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
/// Extra Information about a realm
pub struct ExtraRealmInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issuer_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub autocreate: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username_claim: Option<OpenIdUserAttribute>,
}

#[api(
    properties: {
        "info": {
            type: BasicRealmInfo,
        },
        "extra": {
            type: ExtraRealmInfo,
        },
    },
)]
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
/// Information about a realm
pub struct RealmInfo {
    #[serde(flatten)]
    pub info: BasicRealmInfo,
    #[serde(flatten)]
    pub extra: ExtraRealmInfo,
}

#[api(
    returns: {
        description: "List of realms with basic info.",
        type: Array,
        items: {
            type: BasicRealmInfo,
        }
    },
    access: {
        description: "Anyone can access this, because we need that list for the login box (before the user is authenticated).",
        permission: &Permission::World,
    }
)]
/// Authentication domain/realm index.
fn list_domains(mut rpcenv: &mut dyn RpcEnvironment) -> Result<Vec<BasicRealmInfo>, Error> {
    let mut list = Vec::new();

    list.push(serde_json::from_value(json!({
        "realm": "pam",
        "type": "pam",
        "comment": "Linux PAM standard authentication",
        "default": Some(true),
    }))?);
    list.push(serde_json::from_value(json!({
        "realm": "pbs",
        "type": "pbs",
        "comment": "Proxmox Backup authentication server",
    }))?);

    let (config, digest) = config::domains::config()?;

    for (_, (section_type, v)) in config.sections.iter() {
        let mut entry = v.clone();
        entry["type"] = Value::from(section_type.clone());
        list.push(serde_json::from_value(entry)?);
    }

    rpcenv["digest"] = proxmox::tools::digest_to_hex(&digest).into();

    Ok(list)
}

#[api(
    input: {
        properties: {
            realm: {
                schema: REALM_ID_SCHEMA,
            },
        },
    },
    returns: {
        type: RealmInfo,
    },
    access: {
        permission: &Permission::Privilege(&["access", "domains"], PRIV_SYS_AUDIT | PRIV_REALM_ALLOCATE, true),
    },
)]
/// Get information about a realm
fn get_domain(realm: String, mut rpcenv: &mut dyn RpcEnvironment) -> Result<RealmInfo, Error> {
    let entry = match realm.as_str() {
        "pam" => json!({
            "realm": "pam",
            "type": "pam",
            "comment": "Linux PAM standard authentication",
            "default": Some(true),
        }),
        "pbs" => json!({
            "realm": "pbs",
            "type": "pbs",
            "comment": "Proxmox Backup authentication server",
        }),
        _ => {
            let (config, digest) = config::domains::config()?;
            rpcenv["digest"] = proxmox::tools::digest_to_hex(&digest).into();
            if let Some((section_type, v)) = config.sections.get(&realm) {
                let mut entry = v.clone();
                entry["type"] = Value::from(section_type.clone());
                entry
            } else {
                bail!("domain '{}' does not exist", realm);
            }
        }
    };

    Ok(serde_json::from_value(entry)?)
}

#[api(
    protected: true,
    input: {
        properties: {
            info: {
                type: RealmInfo,
                flatten: true,
            },
        },
    },
    access: {
        permission: &Permission::Privilege(&["access", "domains"], PRIV_REALM_ALLOCATE, false),
    },
)]
/// Create a realm
fn create_domain(param: Value) -> Result<(), Error> {
    let basic_info: BasicRealmInfo = serde_json::from_value(param.clone())?;

    // for now we only have to care about openid
    if basic_info.ty != RealmType::OpenId {
        bail!(
            "Cannot create realm of type '{}'",
            serde_json::to_string(&basic_info.ty)?
        );
    }

    let new_realm: OpenIdRealmConfig = serde_json::from_value(param)?;
    let _lock = config::domains::lock_config()?;

    let (mut config, _digest) = config::domains::config()?;

    let existing: Vec<OpenIdRealmConfig> = config.convert_to_typed_array("openid")?;

    for realm in existing {
        if realm.realm == new_realm.realm {
            bail!("Entry '{}' already exists", realm.realm);
        }
    }

    config.set_data(&new_realm.realm, "openid", &new_realm)?;

    config::domains::save_config(&config)?;

    Ok(())
}

#[api]
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[allow(non_camel_case_types)]
pub enum DeletableProperty {
    /// Delete the comment property.
    comment,
    /// Delete the client-key property.
    client_key,
    /// Delete the autocreate property.
    autocreate,
}

#[api(
    protected: true,
    input: {
        properties: {
            realm: {
                schema: REALM_ID_SCHEMA,
            },
            comment: {
                optional: true,
                schema: SINGLE_LINE_COMMENT_SCHEMA,
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
        permission: &Permission::Privilege(&["access", "domains"], PRIV_REALM_ALLOCATE, false),
    },
)]
/// Update a realm
fn update_domain(
    realm: String,
    comment: Option<String>,
    issuer_url: Option<String>,
    client_id: Option<String>,
    client_key: Option<String>,
    autocreate: Option<bool>,
    delete: Option<Vec<DeletableProperty>>,
    digest: Option<String>,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {
    let _lock = config::domains::lock_config()?;

    let (mut config, expected_digest) = config::domains::config()?;

    if let Some(ref digest) = digest {
        let digest = proxmox::tools::hex_to_digest(digest)?;
        crate::tools::detect_modified_configuration_file(&digest, &expected_digest)?;
    }

    // only have to worry about openid for now
    let mut data: OpenIdRealmConfig = config.lookup("openid", realm.as_str())?;

    if let Some(delete) = delete {
        for delete_prop in delete {
            match delete_prop {
                DeletableProperty::comment => data.comment = None,
                DeletableProperty::client_key => data.client_key = None,
                DeletableProperty::autocreate => data.autocreate = None,
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

    if let Some(issuer_url) = issuer_url {
        data.issuer_url = issuer_url
    };
    if let Some(client_id) = client_id {
        data.client_id = client_id
    };
    if let Some(client_key) = client_key {
        data.client_key = if client_key.is_empty() {
            None
        } else {
            Some(client_key)
        };
    };
    if let Some(autocreate) = autocreate {
        data.autocreate = Some(autocreate)
    };

    config.set_data(&realm, "openid", &data)?;

    config::domains::save_config(&config)?;

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
/// Delete a realm
fn delete_domain(realm: String, digest: Option<String>) -> Result<(), Error> {
    if realm == "pam" || realm == "pbs" {
        bail!("cannot remove realm '{}'", realm);
    }
    let _lock = config::domains::lock_config()?;

    let (mut config, expected_digest) = config::domains::config()?;

    if let Some(ref digest) = digest {
        let digest = proxmox::tools::hex_to_digest(digest)?;
        crate::tools::detect_modified_configuration_file(&digest, &expected_digest)?;
    }

    match config.sections.get(&realm) {
        Some(_) => {
            config.sections.remove(&realm);
        }
        None => bail!("realm '{}' does not exist.", realm),
    }

    config::domains::save_config(&config)?;

    Ok(())
}

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_LIST_DOMAINS)
    .post(&API_METHOD_CREATE_DOMAIN)
    .match_all("realm", &DOMAIN_ROUTER);

const DOMAIN_ROUTER: Router = Router::new()
    .get(&API_METHOD_GET_DOMAIN)
    .put(&API_METHOD_UPDATE_DOMAIN)
    .delete(&API_METHOD_DELETE_DOMAIN);
