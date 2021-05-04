use std::fmt;

use anyhow::Error;
use serde::{Deserialize, Serialize};

use proxmox::api::{api, schema::{Schema, StringSchema, ApiStringFormat}};

use crate::api2::types::{
    DNS_ALIAS_FORMAT, DNS_NAME_FORMAT, PROXMOX_SAFE_ID_FORMAT,
};

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

pub const ACME_DOMAIN_PROPERTY_SCHEMA: Schema = StringSchema::new(
    "ACME domain configuration string")
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

#[api(format: &PROXMOX_SAFE_ID_FORMAT)]
/// ACME account name.
#[derive(Clone, Eq, PartialEq, Hash, Deserialize, Serialize)]
#[serde(transparent)]
pub struct AcmeAccountName(String);

impl AcmeAccountName {
    pub fn into_string(self) -> String {
        self.0
    }

    pub fn from_string(name: String) -> Result<Self, Error> {
        match &Self::API_SCHEMA {
            Schema::String(s) => s.check_constraints(&name)?,
            _ => unreachable!(),
        }
        Ok(Self(name))
    }

    pub unsafe fn from_string_unchecked(name: String) -> Self {
        Self(name)
    }
}

impl std::ops::Deref for AcmeAccountName {
    type Target = str;

    #[inline]
    fn deref(&self) -> &str {
        &self.0
    }
}

impl std::ops::DerefMut for AcmeAccountName {
    #[inline]
    fn deref_mut(&mut self) -> &mut str {
        &mut self.0
    }
}

impl AsRef<str> for AcmeAccountName {
    #[inline]
    fn as_ref(&self) -> &str {
        self.0.as_ref()
    }
}

impl fmt::Debug for AcmeAccountName {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(&self.0, f)
    }
}

impl fmt::Display for AcmeAccountName {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}
