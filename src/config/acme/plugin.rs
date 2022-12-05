use anyhow::Error;
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use proxmox_schema::{api, ApiType, Schema, StringSchema, Updater};
use proxmox_section_config::{SectionConfig, SectionConfigData, SectionConfigPlugin};

use pbs_api_types::PROXMOX_SAFE_ID_FORMAT;
use pbs_config::{open_backup_lockfile, BackupLockGuard};

pub const PLUGIN_ID_SCHEMA: Schema = StringSchema::new("ACME Challenge Plugin ID.")
    .format(&PROXMOX_SAFE_ID_FORMAT)
    .min_length(1)
    .max_length(32)
    .schema();

lazy_static! {
    pub static ref CONFIG: SectionConfig = init();
}

#[api(
    properties: {
        id: { schema: PLUGIN_ID_SCHEMA },
    },
)]
#[derive(Deserialize, Serialize)]
/// Standalone ACME Plugin for the http-1 challenge.
pub struct StandalonePlugin {
    /// Plugin ID.
    id: String,
}

impl Default for StandalonePlugin {
    fn default() -> Self {
        Self {
            id: "standalone".to_string(),
        }
    }
}

#[api(
    properties: {
        id: { schema: PLUGIN_ID_SCHEMA },
        disable: {
            optional: true,
            default: false,
        },
        "validation-delay": {
            default: 30,
            optional: true,
            minimum: 0,
            maximum: 2 * 24 * 60 * 60,
        },
    },
)]
/// DNS ACME Challenge Plugin core data.
#[derive(Deserialize, Serialize, Updater)]
#[serde(rename_all = "kebab-case")]
pub struct DnsPluginCore {
    /// Plugin ID.
    #[updater(skip)]
    pub id: String,

    /// DNS API Plugin Id.
    pub api: String,

    /// Extra delay in seconds to wait before requesting validation.
    ///
    /// Allows to cope with long TTL of DNS records.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub validation_delay: Option<u32>,

    /// Flag to disable the config.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub disable: Option<bool>,
}

#[api(
    properties: {
        core: { type: DnsPluginCore },
    },
)]
/// DNS ACME Challenge Plugin.
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct DnsPlugin {
    #[serde(flatten)]
    pub core: DnsPluginCore,

    // We handle this property separately in the API calls.
    /// DNS plugin data (base64url encoded without padding).
    #[serde(with = "proxmox_serde::string_as_base64url_nopad")]
    pub data: String,
}

impl DnsPlugin {
    pub fn decode_data(&self, output: &mut Vec<u8>) -> Result<(), Error> {
        Ok(base64::decode_config_buf(
            &self.data,
            base64::URL_SAFE_NO_PAD,
            output,
        )?)
    }
}

fn init() -> SectionConfig {
    let mut config = SectionConfig::new(&PLUGIN_ID_SCHEMA);

    let standalone_schema = match &StandalonePlugin::API_SCHEMA {
        Schema::Object(schema) => schema,
        _ => unreachable!(),
    };
    let standalone_plugin = SectionConfigPlugin::new(
        "standalone".to_string(),
        Some("id".to_string()),
        standalone_schema,
    );
    config.register_plugin(standalone_plugin);

    let dns_challenge_schema = match DnsPlugin::API_SCHEMA {
        Schema::AllOf(ref schema) => schema,
        _ => unreachable!(),
    };
    let dns_challenge_plugin = SectionConfigPlugin::new(
        "dns".to_string(),
        Some("id".to_string()),
        dns_challenge_schema,
    );
    config.register_plugin(dns_challenge_plugin);

    config
}

const ACME_PLUGIN_CFG_FILENAME: &str = pbs_buildcfg::configdir!("/acme/plugins.cfg");
const ACME_PLUGIN_CFG_LOCKFILE: &str = pbs_buildcfg::configdir!("/acme/.plugins.lck");

pub fn lock() -> Result<BackupLockGuard, Error> {
    super::make_acme_dir()?;
    open_backup_lockfile(ACME_PLUGIN_CFG_LOCKFILE, None, true)
}

pub fn config() -> Result<(PluginData, [u8; 32]), Error> {
    let content =
        proxmox_sys::fs::file_read_optional_string(ACME_PLUGIN_CFG_FILENAME)?.unwrap_or_default();

    let digest = openssl::sha::sha256(content.as_bytes());
    let mut data = CONFIG.parse(ACME_PLUGIN_CFG_FILENAME, &content)?;

    if data.sections.get("standalone").is_none() {
        let standalone = StandalonePlugin::default();
        data.set_data("standalone", "standalone", &standalone)
            .unwrap();
    }

    Ok((PluginData { data }, digest))
}

pub fn save_config(config: &PluginData) -> Result<(), Error> {
    super::make_acme_dir()?;
    let raw = CONFIG.write(ACME_PLUGIN_CFG_FILENAME, &config.data)?;
    pbs_config::replace_backup_config(ACME_PLUGIN_CFG_FILENAME, raw.as_bytes())
}

pub struct PluginData {
    data: SectionConfigData,
}

// And some convenience helpers.
impl PluginData {
    pub fn remove(&mut self, name: &str) -> Option<(String, Value)> {
        self.data.sections.remove(name)
    }

    pub fn contains_key(&mut self, name: &str) -> bool {
        self.data.sections.contains_key(name)
    }

    pub fn get(&self, name: &str) -> Option<&(String, Value)> {
        self.data.sections.get(name)
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut (String, Value)> {
        self.data.sections.get_mut(name)
    }

    pub fn insert(&mut self, id: String, ty: String, plugin: Value) {
        self.data.sections.insert(id, (ty, plugin));
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &(String, Value))> + Send {
        self.data.sections.iter()
    }
}
