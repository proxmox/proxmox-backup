//! List Authentication domains/realms

use anyhow::{Error};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use proxmox::api::{api, Permission, Router, RpcEnvironment};

use crate::config;
use crate::api2::types::*;
use crate::config::domains::{OpenIdRealmConfig, OpenIdUserAttribute};

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

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_LIST_DOMAINS);
