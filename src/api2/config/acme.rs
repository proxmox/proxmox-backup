use std::fs;
use std::ops::ControlFlow;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use anyhow::{bail, format_err, Error};
use hex::FromHex;
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use proxmox_router::{
    http_bail, list_subdirs_api_method, Permission, Router, RpcEnvironment, SubdirMap,
};
use proxmox_schema::{api, param_bail};
use proxmox_sys::{task_log, task_warn};

use proxmox_acme::account::AccountData as AcmeAccountData;
use proxmox_acme::Account;

use pbs_api_types::{Authid, PRIV_SYS_MODIFY};

use crate::acme::AcmeClient;
use crate::api2::types::{AcmeAccountName, AcmeChallengeSchema, KnownAcmeDirectory};
use crate::config::acme::plugin::{
    self, DnsPlugin, DnsPluginCore, DnsPluginCoreUpdater, PLUGIN_ID_SCHEMA,
};
use proxmox_rest_server::WorkerTask;

pub(crate) const ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(SUBDIRS))
    .subdirs(SUBDIRS);

const SUBDIRS: SubdirMap = &[
    (
        "account",
        &Router::new()
            .get(&API_METHOD_LIST_ACCOUNTS)
            .post(&API_METHOD_REGISTER_ACCOUNT)
            .match_all("name", &ACCOUNT_ITEM_ROUTER),
    ),
    (
        "challenge-schema",
        &Router::new().get(&API_METHOD_GET_CHALLENGE_SCHEMA),
    ),
    (
        "directories",
        &Router::new().get(&API_METHOD_GET_DIRECTORIES),
    ),
    (
        "plugins",
        &Router::new()
            .get(&API_METHOD_LIST_PLUGINS)
            .post(&API_METHOD_ADD_PLUGIN)
            .match_all("id", &PLUGIN_ITEM_ROUTER),
    ),
    ("tos", &Router::new().get(&API_METHOD_GET_TOS)),
];

const ACCOUNT_ITEM_ROUTER: Router = Router::new()
    .get(&API_METHOD_GET_ACCOUNT)
    .put(&API_METHOD_UPDATE_ACCOUNT)
    .delete(&API_METHOD_DEACTIVATE_ACCOUNT);

const PLUGIN_ITEM_ROUTER: Router = Router::new()
    .get(&API_METHOD_GET_PLUGIN)
    .put(&API_METHOD_UPDATE_PLUGIN)
    .delete(&API_METHOD_DELETE_PLUGIN);

#[api(
    properties: {
        name: { type: AcmeAccountName },
    },
)]
/// An ACME Account entry.
///
/// Currently only contains a 'name' property.
#[derive(Serialize)]
pub struct AccountEntry {
    name: AcmeAccountName,
}

#[api(
    access: {
        permission: &Permission::Privilege(&["system", "certificates"], PRIV_SYS_MODIFY, false),
    },
    returns: {
        type: Array,
        items: { type: AccountEntry },
        description: "List of ACME accounts.",
    },
    protected: true,
)]
/// List ACME accounts.
pub fn list_accounts() -> Result<Vec<AccountEntry>, Error> {
    let mut entries = Vec::new();
    crate::config::acme::foreach_acme_account(|name| {
        entries.push(AccountEntry { name });
        ControlFlow::Continue(())
    })?;
    Ok(entries)
}

#[api(
    properties: {
        account: { type: Object, properties: {}, additional_properties: true },
        tos: {
            type: String,
            optional: true,
        },
    },
)]
/// ACME Account information.
///
/// This is what we return via the API.
#[derive(Serialize)]
pub struct AccountInfo {
    /// Raw account data.
    account: AcmeAccountData,

    /// The ACME directory URL the account was created at.
    directory: String,

    /// The account's own URL within the ACME directory.
    location: String,

    /// The ToS URL, if the user agreed to one.
    #[serde(skip_serializing_if = "Option::is_none")]
    tos: Option<String>,
}

#[api(
    input: {
        properties: {
            name: { type: AcmeAccountName },
        },
    },
    access: {
        permission: &Permission::Privilege(&["system", "certificates"], PRIV_SYS_MODIFY, false),
    },
    returns: { type: AccountInfo },
    protected: true,
)]
/// Return existing ACME account information.
pub async fn get_account(name: AcmeAccountName) -> Result<AccountInfo, Error> {
    let client = AcmeClient::load(&name).await?;
    let account = client.account()?;
    Ok(AccountInfo {
        location: account.location.clone(),
        tos: client.tos().map(str::to_owned),
        directory: client.directory_url().to_owned(),
        account: AcmeAccountData {
            only_return_existing: false, // don't actually write this out in case it's set
            ..account.data.clone()
        },
    })
}

fn account_contact_from_string(s: &str) -> Vec<String> {
    s.split(&[' ', ';', ',', '\0'][..])
        .map(|s| format!("mailto:{}", s))
        .collect()
}

#[api(
    input: {
        properties: {
            name: {
                type: AcmeAccountName,
                optional: true,
            },
            contact: {
                description: "List of email addresses.",
            },
            tos_url: {
                description: "URL of CA TermsOfService - setting this indicates agreement.",
                optional: true,
            },
            directory: {
                type: String,
                description: "The ACME Directory.",
                optional: true,
            },
            eab_kid: {
                type: String,
                description: "Key Identifier for External Account Binding.",
                optional: true,
            },
            eab_hmac_key: {
                type: String,
                description: "HMAC Key for External Account Binding.",
                optional: true,
            }
        },
    },
    access: {
        permission: &Permission::Privilege(&["system", "certificates"], PRIV_SYS_MODIFY, false),
    },
    protected: true,
)]
/// Register an ACME account.
fn register_account(
    name: Option<AcmeAccountName>,
    // Todo: email & email-list schema
    contact: String,
    tos_url: Option<String>,
    directory: Option<String>,
    eab_kid: Option<String>,
    eab_hmac_key: Option<String>,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<String, Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;

    let name = name.unwrap_or_else(|| unsafe {
        AcmeAccountName::from_string_unchecked("default".to_string())
    });

    // TODO: this should be done via the api definition, but
    // the api schema currently lacks this ability (2023-11-06)
    if eab_kid.is_some() != eab_hmac_key.is_some() {
        http_bail!(
            BAD_REQUEST,
            "either both or none of 'eab_kid' and 'eab_hmac_key' have to be set."
        );
    }

    if Path::new(&crate::config::acme::account_path(&name)).exists() {
        http_bail!(BAD_REQUEST, "account {} already exists", name);
    }

    let directory = directory.unwrap_or_else(|| {
        crate::config::acme::DEFAULT_ACME_DIRECTORY_ENTRY
            .url
            .to_owned()
    });

    WorkerTask::spawn(
        "acme-register",
        Some(name.to_string()),
        auth_id.to_string(),
        true,
        move |worker| async move {
            let mut client = AcmeClient::new(directory);

            task_log!(worker, "Registering ACME account '{}'...", &name);

            let account = do_register_account(
                &mut client,
                &name,
                tos_url.is_some(),
                contact,
                None,
                eab_kid.zip(eab_hmac_key),
            )
            .await?;

            task_log!(
                worker,
                "Registration successful, account URL: {}",
                account.location
            );

            Ok(())
        },
    )
}

pub async fn do_register_account<'a>(
    client: &'a mut AcmeClient,
    name: &AcmeAccountName,
    agree_to_tos: bool,
    contact: String,
    rsa_bits: Option<u32>,
    eab_creds: Option<(String, String)>,
) -> Result<&'a Account, Error> {
    let contact = account_contact_from_string(&contact);
    client
        .new_account(name, agree_to_tos, contact, rsa_bits, eab_creds)
        .await
}

#[api(
    input: {
        properties: {
            name: { type: AcmeAccountName },
            contact: {
                description: "List of email addresses.",
                optional: true,
            },
        },
    },
    access: {
        permission: &Permission::Privilege(&["system", "certificates"], PRIV_SYS_MODIFY, false),
    },
    protected: true,
)]
/// Update an ACME account.
pub fn update_account(
    name: AcmeAccountName,
    // Todo: email & email-list schema
    contact: Option<String>,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<String, Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;

    WorkerTask::spawn(
        "acme-update",
        Some(name.to_string()),
        auth_id.to_string(),
        true,
        move |_worker| async move {
            let data = match contact {
                Some(data) => json!({
                    "contact": account_contact_from_string(&data),
                }),
                None => json!({}),
            };

            AcmeClient::load(&name).await?.update_account(&data).await?;

            Ok(())
        },
    )
}

#[api(
    input: {
        properties: {
            name: { type: AcmeAccountName },
            force: {
                description:
                    "Delete account data even if the server refuses to deactivate the account.",
                optional: true,
                default: false,
            },
        },
    },
    access: {
        permission: &Permission::Privilege(&["system", "certificates"], PRIV_SYS_MODIFY, false),
    },
    protected: true,
)]
/// Deactivate an ACME account.
pub fn deactivate_account(
    name: AcmeAccountName,
    force: bool,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<String, Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;

    WorkerTask::spawn(
        "acme-deactivate",
        Some(name.to_string()),
        auth_id.to_string(),
        true,
        move |worker| async move {
            match AcmeClient::load(&name)
                .await?
                .update_account(&json!({"status": "deactivated"}))
                .await
            {
                Ok(_account) => (),
                Err(err) if !force => return Err(err),
                Err(err) => {
                    task_warn!(
                        worker,
                        "error deactivating account {}, proceedeing anyway - {}",
                        name,
                        err,
                    );
                }
            }
            crate::config::acme::mark_account_deactivated(&name)?;
            Ok(())
        },
    )
}

#[api(
    input: {
        properties: {
            directory: {
                type: String,
                description: "The ACME Directory.",
                optional: true,
            },
        },
    },
    access: {
        permission: &Permission::Anybody,
    },
    returns: {
        type: String,
        optional: true,
        description: "The ACME Directory's ToS URL, if any.",
    },
)]
/// Get the Terms of Service URL for an ACME directory.
async fn get_tos(directory: Option<String>) -> Result<Option<String>, Error> {
    let directory = directory.unwrap_or_else(|| {
        crate::config::acme::DEFAULT_ACME_DIRECTORY_ENTRY
            .url
            .to_owned()
    });
    Ok(AcmeClient::new(directory)
        .terms_of_service_url()
        .await?
        .map(str::to_owned))
}

#[api(
    access: {
        permission: &Permission::Anybody,
    },
    returns: {
        description: "List of known ACME directories.",
        type: Array,
        items: { type: KnownAcmeDirectory },
    },
)]
/// Get named known ACME directory endpoints.
fn get_directories() -> Result<&'static [KnownAcmeDirectory], Error> {
    Ok(crate::config::acme::KNOWN_ACME_DIRECTORIES)
}

/// Wrapper for efficient Arc use when returning the ACME challenge-plugin schema for serializing
struct ChallengeSchemaWrapper {
    inner: Arc<Vec<AcmeChallengeSchema>>,
}

impl Serialize for ChallengeSchemaWrapper {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.inner.serialize(serializer)
    }
}

fn get_cached_challenge_schemas() -> Result<ChallengeSchemaWrapper, Error> {
    lazy_static! {
        static ref CACHE: Mutex<Option<(Arc<Vec<AcmeChallengeSchema>>, SystemTime)>> =
            Mutex::new(None);
    }

    // the actual loading code
    let mut last = CACHE.lock().unwrap();

    let actual_mtime = fs::metadata(crate::config::acme::ACME_DNS_SCHEMA_FN)?.modified()?;

    let schema = match &*last {
        Some((schema, cached_mtime)) if *cached_mtime >= actual_mtime => schema.clone(),
        _ => {
            let new_schema = Arc::new(crate::config::acme::load_dns_challenge_schema()?);
            *last = Some((Arc::clone(&new_schema), actual_mtime));
            new_schema
        }
    };

    Ok(ChallengeSchemaWrapper { inner: schema })
}

#[api(
    access: {
        permission: &Permission::Anybody,
    },
    returns: {
        description: "ACME Challenge Plugin Shema.",
        type: Array,
        items: { type: AcmeChallengeSchema },
    },
)]
/// Get named known ACME directory endpoints.
fn get_challenge_schema() -> Result<ChallengeSchemaWrapper, Error> {
    get_cached_challenge_schemas()
}

#[api]
#[derive(Default, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
/// The API's format is inherited from PVE/PMG:
pub struct PluginConfig {
    /// Plugin ID.
    plugin: String,

    /// Plugin type.
    #[serde(rename = "type")]
    ty: String,

    /// DNS Api name.
    api: Option<String>,

    /// Plugin configuration data.
    data: Option<String>,

    /// Extra delay in seconds to wait before requesting validation.
    ///
    /// Allows to cope with long TTL of DNS records.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    validation_delay: Option<u32>,

    /// Flag to disable the config.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    disable: Option<bool>,
}

// See PMG/PVE's $modify_cfg_for_api sub
fn modify_cfg_for_api(id: &str, ty: &str, data: &Value) -> PluginConfig {
    let mut entry = data.clone();

    let obj = entry.as_object_mut().unwrap();
    obj.remove("id");
    obj.insert("plugin".to_string(), Value::String(id.to_owned()));
    obj.insert("type".to_string(), Value::String(ty.to_owned()));

    // FIXME: This needs to go once the `Updater` is fixed.
    // None of these should be able to fail unless the user changed the files by hand, in which
    // case we leave the unmodified string in the Value for now. This will be handled with an error
    // later.
    if let Some(Value::String(ref mut data)) = obj.get_mut("data") {
        if let Ok(new) = base64::decode_config(&data, base64::URL_SAFE_NO_PAD) {
            if let Ok(utf8) = String::from_utf8(new) {
                *data = utf8;
            }
        }
    }

    // PVE/PMG do this explicitly for ACME plugins...
    // obj.insert("digest".to_string(), Value::String(digest.clone()));

    serde_json::from_value(entry).unwrap_or_else(|_| PluginConfig {
        plugin: "*Error*".to_string(),
        ty: "*Error*".to_string(),
        ..Default::default()
    })
}

#[api(
    access: {
        permission: &Permission::Privilege(&["system", "certificates"], PRIV_SYS_MODIFY, false),
    },
    protected: true,
    returns: {
        type: Array,
        description: "List of ACME plugin configurations.",
        items: { type: PluginConfig },
    },
)]
/// List ACME challenge plugins.
pub fn list_plugins(rpcenv: &mut dyn RpcEnvironment) -> Result<Vec<PluginConfig>, Error> {
    let (plugins, digest) = plugin::config()?;
    rpcenv["digest"] = hex::encode(digest).into();
    Ok(plugins
        .iter()
        .map(|(id, (ty, data))| modify_cfg_for_api(id, ty, data))
        .collect())
}

#[api(
    input: {
        properties: {
            id: { schema: PLUGIN_ID_SCHEMA },
        },
    },
    access: {
        permission: &Permission::Privilege(&["system", "certificates"], PRIV_SYS_MODIFY, false),
    },
    protected: true,
    returns: { type: PluginConfig },
)]
/// List ACME challenge plugins.
pub fn get_plugin(id: String, rpcenv: &mut dyn RpcEnvironment) -> Result<PluginConfig, Error> {
    let (plugins, digest) = plugin::config()?;
    rpcenv["digest"] = hex::encode(digest).into();

    match plugins.get(&id) {
        Some((ty, data)) => Ok(modify_cfg_for_api(&id, ty, data)),
        None => http_bail!(NOT_FOUND, "no such plugin"),
    }
}

// Currently we only have "the" standalone plugin and DNS plugins so we can just flatten a
// DnsPluginUpdater:
//
// FIXME: The 'id' parameter should not be "optional" in the schema.
#[api(
    input: {
        properties: {
            type: {
                type: String,
                description: "The ACME challenge plugin type.",
            },
            core: {
                type: DnsPluginCore,
                flatten: true,
            },
            data: {
                type: String,
                // This is different in the API!
                description: "DNS plugin data (base64 encoded with padding).",
            },
        },
    },
    access: {
        permission: &Permission::Privilege(&["system", "certificates"], PRIV_SYS_MODIFY, false),
    },
    protected: true,
)]
/// Add ACME plugin configuration.
pub fn add_plugin(r#type: String, core: DnsPluginCore, data: String) -> Result<(), Error> {
    // Currently we only support DNS plugins and the standalone plugin is "fixed":
    if r#type != "dns" {
        param_bail!("type", "invalid ACME plugin type: {:?}", r#type);
    }

    let data = String::from_utf8(base64::decode(data)?)
        .map_err(|_| format_err!("data must be valid UTF-8"))?;

    let id = core.id.clone();

    let _lock = plugin::lock()?;

    let (mut plugins, _digest) = plugin::config()?;
    if plugins.contains_key(&id) {
        param_bail!("id", "ACME plugin ID {:?} already exists", id);
    }

    let plugin = serde_json::to_value(DnsPlugin { core, data })?;

    plugins.insert(id, r#type, plugin);

    plugin::save_config(&plugins)?;

    Ok(())
}

#[api(
    input: {
        properties: {
            id: { schema: PLUGIN_ID_SCHEMA },
        },
    },
    access: {
        permission: &Permission::Privilege(&["system", "certificates"], PRIV_SYS_MODIFY, false),
    },
    protected: true,
)]
/// Delete an ACME plugin configuration.
pub fn delete_plugin(id: String) -> Result<(), Error> {
    let _lock = plugin::lock()?;

    let (mut plugins, _digest) = plugin::config()?;
    if plugins.remove(&id).is_none() {
        http_bail!(NOT_FOUND, "no such plugin");
    }
    plugin::save_config(&plugins)?;

    Ok(())
}

#[api()]
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
/// Deletable property name
pub enum DeletableProperty {
    /// Delete the disable property
    Disable,
    /// Delete the validation-delay property
    ValidationDelay,
}

#[api(
    input: {
        properties: {
            id: { schema: PLUGIN_ID_SCHEMA },
            update: {
                type: DnsPluginCoreUpdater,
                flatten: true,
            },
            data: {
                type: String,
                optional: true,
                // This is different in the API!
                description: "DNS plugin data (base64 encoded with padding).",
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
                description: "Digest to protect against concurrent updates",
                optional: true,
            },
        },
    },
    access: {
        permission: &Permission::Privilege(&["system", "certificates"], PRIV_SYS_MODIFY, false),
    },
    protected: true,
)]
/// Update an ACME plugin configuration.
pub fn update_plugin(
    id: String,
    update: DnsPluginCoreUpdater,
    data: Option<String>,
    delete: Option<Vec<DeletableProperty>>,
    digest: Option<String>,
) -> Result<(), Error> {
    let data = data
        .as_deref()
        .map(base64::decode)
        .transpose()?
        .map(String::from_utf8)
        .transpose()
        .map_err(|_| format_err!("data must be valid UTF-8"))?;

    let _lock = plugin::lock()?;

    let (mut plugins, expected_digest) = plugin::config()?;

    if let Some(digest) = digest {
        let digest = <[u8; 32]>::from_hex(digest)?;
        crate::tools::detect_modified_configuration_file(&digest, &expected_digest)?;
    }

    match plugins.get_mut(&id) {
        Some((ty, ref mut entry)) => {
            if ty != "dns" {
                bail!("cannot update plugin of type {:?}", ty);
            }

            let mut plugin = DnsPlugin::deserialize(&*entry)?;

            if let Some(delete) = delete {
                for delete_prop in delete {
                    match delete_prop {
                        DeletableProperty::ValidationDelay => {
                            plugin.core.validation_delay = None;
                        }
                        DeletableProperty::Disable => {
                            plugin.core.disable = None;
                        }
                    }
                }
            }
            if let Some(data) = data {
                plugin.data = data;
            }
            if let Some(api) = update.api {
                plugin.core.api = api;
            }
            if update.validation_delay.is_some() {
                plugin.core.validation_delay = update.validation_delay;
            }
            if update.disable.is_some() {
                plugin.core.disable = update.disable;
            }

            *entry = serde_json::to_value(plugin)?;
        }
        None => http_bail!(NOT_FOUND, "no such plugin"),
    }

    plugin::save_config(&plugins)?;

    Ok(())
}
