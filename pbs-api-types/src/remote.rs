use serde::{Deserialize, Serialize};

use super::*;
use proxmox_schema::*;

pub const REMOTE_PASSWORD_SCHEMA: Schema =
    StringSchema::new("Password or auth token for remote host.")
        .format(&PASSWORD_FORMAT)
        .min_length(1)
        .max_length(1024)
        .schema();

pub const REMOTE_PASSWORD_BASE64_SCHEMA: Schema =
    StringSchema::new("Password or auth token for remote host (stored as base64 string).")
        .format(&PASSWORD_FORMAT)
        .min_length(1)
        .max_length(1024)
        .schema();

pub const REMOTE_ID_SCHEMA: Schema = StringSchema::new("Remote ID.")
    .format(&PROXMOX_SAFE_ID_FORMAT)
    .min_length(3)
    .max_length(32)
    .schema();

#[api(
    properties: {
        comment: {
            optional: true,
            schema: SINGLE_LINE_COMMENT_SCHEMA,
        },
        host: {
            schema: DNS_NAME_OR_IP_SCHEMA,
        },
        port: {
            optional: true,
            description: "The (optional) port",
            type: u16,
        },
        "auth-id": {
            type: Authid,
        },
        fingerprint: {
            optional: true,
            schema: CERT_FINGERPRINT_SHA256_SCHEMA,
        },
    },
)]
#[derive(Serialize, Deserialize, Updater, Clone, PartialEq)]
#[serde(rename_all = "kebab-case")]
/// Remote configuration properties.
pub struct RemoteConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    pub host: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    pub auth_id: Authid,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
}

#[api(
    properties: {
        name: {
            schema: REMOTE_ID_SCHEMA,
        },
        config: {
            type: RemoteConfig,
        },
        password: {
            schema: REMOTE_PASSWORD_BASE64_SCHEMA,
        },
    },
)]
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
/// Remote properties.
pub struct Remote {
    pub name: String,
    // Note: The stored password is base64 encoded
    #[serde(default, skip_serializing_if = "String::is_empty")]
    #[serde(with = "proxmox_serde::string_as_base64")]
    pub password: String,
    #[serde(flatten)]
    pub config: RemoteConfig,
}

#[api(
    properties: {
        name: {
            schema: REMOTE_ID_SCHEMA,
        },
        config: {
            type: RemoteConfig,
        },
    },
)]
#[derive(Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "kebab-case")]
/// Remote properties.
pub struct RemoteWithoutPassword {
    pub name: String,
    #[serde(flatten)]
    pub config: RemoteConfig,
}
