use serde::{Deserialize, Serialize};
use serde_json::Value;

use proxmox_schema::{api, ApiStringFormat, ApiType, Schema, StringSchema};

use pbs_api_types::{DNS_ALIAS_FORMAT, DNS_NAME_FORMAT, PROXMOX_SAFE_ID_FORMAT};

#[api(
    properties: {
        "domain": { format: &DNS_NAME_FORMAT },
        "alias": {
            optional: true,
            format: &DNS_ALIAS_FORMAT,
        },
        "plugin": {
            optional: true,
            format: &PROXMOX_SAFE_ID_FORMAT,
        },
    },
    default_key: "domain",
)]
#[derive(Deserialize, Serialize)]
/// A domain entry for an ACME certificate.
pub struct AcmeDomain {
    /// The domain to certify for.
    pub domain: String,

    /// The domain to use for challenges instead of the default acme challenge domain.
    ///
    /// This is useful if you use CNAME entries to redirect `_acme-challenge.*` domains to a
    /// different DNS server.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,

    /// The plugin to use to validate this domain.
    ///
    /// Empty means standalone HTTP validation is used.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plugin: Option<String>,
}

pub const ACME_DOMAIN_PROPERTY_SCHEMA: Schema =
    StringSchema::new("ACME domain configuration string")
        .format(&ApiStringFormat::PropertyString(&AcmeDomain::API_SCHEMA))
        .schema();

#[api(
    properties: {
        name: { type: String },
        url: { type: String },
    },
)]
/// An ACME directory endpoint with a name and URL.
#[derive(Serialize)]
pub struct KnownAcmeDirectory {
    /// The ACME directory's name.
    pub name: &'static str,

    /// The ACME directory's endpoint URL.
    pub url: &'static str,
}

proxmox_schema::api_string_type! {
    #[api(format: &PROXMOX_SAFE_ID_FORMAT)]
    /// ACME account name.
    #[derive(Clone, Eq, PartialEq, Hash, Deserialize, Serialize)]
    #[serde(transparent)]
    pub struct AcmeAccountName(String);
}

#[api(
    properties: {
        schema: {
            type: Object,
            additional_properties: true,
            properties: {},
        },
        type: {
            type: String,
        },
    },
)]
#[derive(Serialize)]
/// Schema for an ACME challenge plugin.
pub struct AcmeChallengeSchema {
    /// Plugin ID.
    pub id: String,

    /// Human readable name, falls back to id.
    pub name: String,

    /// Plugin Type.
    #[serde(rename = "type")]
    pub ty: &'static str,

    /// The plugin's parameter schema.
    pub schema: Value,
}
