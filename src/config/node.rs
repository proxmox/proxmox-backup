use std::collections::HashSet;

use anyhow::{bail, Error};
use serde::{Deserialize, Serialize};

use proxmox::api::api;
use proxmox::api::schema::{ApiStringFormat, Updater};

use proxmox_http::ProxyConfig;

use pbs_buildcfg::configdir;

use crate::backup::{open_backup_lockfile, BackupLockGuard};
use crate::acme::AcmeClient;
use crate::api2::types::{
    AcmeAccountName, AcmeDomain, ACME_DOMAIN_PROPERTY_SCHEMA, HTTP_PROXY_SCHEMA,
};

const CONF_FILE: &str = configdir!("/node.cfg");
const LOCK_FILE: &str = configdir!("/.node.lck");

pub fn lock() -> Result<BackupLockGuard, Error> {
    open_backup_lockfile(LOCK_FILE, None, true)
}

/// Read the Node Config.
pub fn config() -> Result<(NodeConfig, [u8; 32]), Error> {
    let content =
        proxmox::tools::fs::file_read_optional_string(CONF_FILE)?.unwrap_or_else(|| "".to_string());

    let digest = openssl::sha::sha256(content.as_bytes());
    let data: NodeConfig = crate::tools::config::from_str(&content, &NodeConfig::API_SCHEMA)?;

    Ok((data, digest))
}

/// Write the Node Config, requires the write lock to be held.
pub fn save_config(config: &NodeConfig) -> Result<(), Error> {
    config.validate()?;

    let raw = crate::tools::config::to_bytes(config, &NodeConfig::API_SCHEMA)?;
    crate::backup::replace_backup_config(CONF_FILE, &raw)
}

#[api(
    properties: {
        account: { type: AcmeAccountName },
    }
)]
#[derive(Deserialize, Serialize)]
/// The ACME configuration.
///
/// Currently only contains the name of the account use.
pub struct AcmeConfig {
    /// Account to use to acquire ACME certificates.
    account: AcmeAccountName,
}

#[api(
    properties: {
        acme: {
            optional: true,
            type: String,
            format: &ApiStringFormat::PropertyString(&AcmeConfig::API_SCHEMA),
        },
        acmedomain0: {
            schema: ACME_DOMAIN_PROPERTY_SCHEMA,
            optional: true,
        },
        acmedomain1: {
            schema: ACME_DOMAIN_PROPERTY_SCHEMA,
            optional: true,
        },
        acmedomain2: {
            schema: ACME_DOMAIN_PROPERTY_SCHEMA,
            optional: true,
        },
        acmedomain3: {
            schema: ACME_DOMAIN_PROPERTY_SCHEMA,
            optional: true,
        },
        acmedomain4: {
            schema: ACME_DOMAIN_PROPERTY_SCHEMA,
            optional: true,
        },
        "http-proxy": {
            schema: HTTP_PROXY_SCHEMA,
            optional: true,
        },
    },
)]
#[derive(Deserialize, Serialize, Updater)]
#[serde(rename_all = "kebab-case")]
/// Node specific configuration.
pub struct NodeConfig {
    /// The acme account to use on this node.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub acme: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub acmedomain0: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub acmedomain1: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub acmedomain2: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub acmedomain3: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub acmedomain4: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub http_proxy: Option<String>,
}

impl NodeConfig {
    pub fn acme_config(&self) -> Option<Result<AcmeConfig, Error>> {
        self.acme.as_deref().map(|config| -> Result<_, Error> {
            Ok(crate::tools::config::from_property_string(
                config,
                &AcmeConfig::API_SCHEMA,
            )?)
        })
    }

    pub async fn acme_client(&self) -> Result<AcmeClient, Error> {
        let account = if let Some(cfg) = self.acme_config().transpose()? {
            cfg.account
        } else {
            AcmeAccountName::from_string("default".to_string())? // should really not happen
        };
        AcmeClient::load(&account).await
    }

    pub fn acme_domains(&self) -> AcmeDomainIter {
        AcmeDomainIter::new(self)
    }

    /// Returns the parsed ProxyConfig
    pub fn http_proxy(&self) -> Option<ProxyConfig> {
        if let Some(http_proxy) = &self.http_proxy {
            match ProxyConfig::parse_proxy_url(&http_proxy) {
                Ok(proxy) => Some(proxy),
                Err(_) => None,
            }
        } else {
            None
        }
    }

    /// Sets the HTTP proxy configuration
    pub fn set_http_proxy(&mut self, http_proxy: Option<String>) {
        self.http_proxy = http_proxy;
    }

    /// Validate the configuration.
    pub fn validate(&self) -> Result<(), Error> {
        let mut domains = HashSet::new();
        for domain in self.acme_domains() {
            let domain = domain?;
            if !domains.insert(domain.domain.to_lowercase()) {
                bail!("duplicate domain '{}' in ACME config", domain.domain);
            }
        }

        Ok(())
    }
}

pub struct AcmeDomainIter<'a> {
    config: &'a NodeConfig,
    index: usize,
}

impl<'a> AcmeDomainIter<'a> {
    fn new(config: &'a NodeConfig) -> Self {
        Self { config, index: 0 }
    }
}

impl<'a> Iterator for AcmeDomainIter<'a> {
    type Item = Result<AcmeDomain, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        let domain = loop {
            let index = self.index;
            self.index += 1;

            let domain = match index {
                0 => self.config.acmedomain0.as_deref(),
                1 => self.config.acmedomain1.as_deref(),
                2 => self.config.acmedomain2.as_deref(),
                3 => self.config.acmedomain3.as_deref(),
                4 => self.config.acmedomain4.as_deref(),
                _ => return None,
            };

            if let Some(domain) = domain {
                break domain;
            }
        };

        Some(crate::tools::config::from_property_string(
            domain,
            &AcmeDomain::API_SCHEMA,
        ))
    }
}
