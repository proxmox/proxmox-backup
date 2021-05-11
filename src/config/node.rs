use std::collections::HashSet;
use std::fs::File;
use std::time::Duration;

use anyhow::{bail, Error};
use nix::sys::stat::Mode;
use serde::{Deserialize, Serialize};

use proxmox::api::api;
use proxmox::api::schema::{ApiStringFormat, Updater};
use proxmox::tools::fs::{replace_file, CreateOptions};

use crate::acme::AcmeClient;
use crate::api2::types::{
    AcmeAccountName, AcmeDomain, ACME_DOMAIN_PROPERTY_SCHEMA, HTTP_PROXY_SCHEMA,
};
use crate::tools::http::ProxyConfig;

const CONF_FILE: &str = configdir!("/node.cfg");
const LOCK_FILE: &str = configdir!("/.node.lck");
const LOCK_TIMEOUT: Duration = Duration::from_secs(10);

pub fn lock() -> Result<File, Error> {
    proxmox::tools::fs::open_file_locked(LOCK_FILE, LOCK_TIMEOUT, true)
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

    let backup_user = crate::backup::backup_user()?;
    let options = CreateOptions::new()
        .perm(Mode::from_bits_truncate(0o0640))
        .owner(nix::unistd::ROOT)
        .group(backup_user.gid);

    replace_file(CONF_FILE, &raw, options)
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
    #[serde(skip_serializing_if = "Updater::is_empty")]
    acme: Option<String>,

    #[serde(skip_serializing_if = "Updater::is_empty")]
    acmedomain0: Option<String>,

    #[serde(skip_serializing_if = "Updater::is_empty")]
    acmedomain1: Option<String>,

    #[serde(skip_serializing_if = "Updater::is_empty")]
    acmedomain2: Option<String>,

    #[serde(skip_serializing_if = "Updater::is_empty")]
    acmedomain3: Option<String>,

    #[serde(skip_serializing_if = "Updater::is_empty")]
    acmedomain4: Option<String>,

    #[serde(skip_serializing_if = "Updater::is_empty")]
    http_proxy: Option<String>,
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

    pub fn set_proxy(&mut self, http_proxy: Option<String>) {
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
