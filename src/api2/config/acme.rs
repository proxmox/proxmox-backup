use std::path::Path;

use anyhow::{bail, format_err, Error};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use proxmox::api::router::SubdirMap;
use proxmox::api::schema::Updatable;
use proxmox::api::{api, Permission, Router, RpcEnvironment};
use proxmox::http_bail;
use proxmox::list_subdirs_api_method;

use proxmox_acme_rs::account::AccountData as AcmeAccountData;
use proxmox_acme_rs::Account;

use crate::acme::AcmeClient;
use crate::config::acl::PRIV_SYS_MODIFY;
use crate::config::acme::plugin::{
    DnsPlugin, DnsPluginCore, DnsPluginCoreUpdater, PLUGIN_ID_SCHEMA,
};
use crate::api2::types::{Authid, KnownAcmeDirectory, AcmeAccountName};
use crate::server::WorkerTask;
use crate::tools::ControlFlow;

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
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<String, Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;

    let name = name
        .unwrap_or_else(|| unsafe { AcmeAccountName::from_string_unchecked("default".to_string()) });

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
        None,
        auth_id,
        true,
        move |worker| async move {
            let mut client = AcmeClient::new(directory);

            worker.log("Registering ACME account...");

            let account =
                do_register_account(&mut client, &name, tos_url.is_some(), contact, None).await?;

            worker.log(format!(
                "Registration successful, account URL: {}",
                account.location
            ));

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
) -> Result<&'a Account, Error> {
    let contact = account_contact_from_string(&contact);
    Ok(client
        .new_account(name, agree_to_tos, contact, rsa_bits)
        .await?)
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
        None,
        auth_id,
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
        None,
        auth_id,
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
                    worker.warn(format!(
                        "error deactivating account {}, proceedeing anyway - {}",
                        name, err,
                    ));
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
pub struct ChallengeSchema {
    /// Plugin ID.
    id: String,

    /// Human readable name, falls back to id.
    name: String,

    /// Plugin Type.
    #[serde(rename = "type")]
    ty: &'static str,

    /// The plugin's parameter schema.
    schema: Value,
}

#[api(
    access: {
        permission: &Permission::Anybody,
    },
    returns: {
        description: "ACME Challenge Plugin Shema.",
        type: Array,
        items: { type: ChallengeSchema },
    },
)]
/// Get named known ACME directory endpoints.
fn get_challenge_schema() -> Result<Vec<ChallengeSchema>, Error> {
    let mut out = Vec::new();
    crate::config::acme::foreach_dns_plugin(|id| {
        out.push(ChallengeSchema {
            id: id.to_owned(),
            name: id.to_owned(),
            ty: "dns",
            schema: Value::Object(Default::default()),
        });
        ControlFlow::Continue(())
    })?;
    Ok(out)
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
pub fn list_plugins(mut rpcenv: &mut dyn RpcEnvironment) -> Result<Vec<PluginConfig>, Error> {
    use crate::config::acme::plugin;

    let (plugins, digest) = plugin::config()?;
    rpcenv["digest"] = proxmox::tools::digest_to_hex(&digest).into();
    Ok(plugins
        .iter()
        .map(|(id, (ty, data))| modify_cfg_for_api(&id, &ty, data))
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
pub fn get_plugin(id: String, mut rpcenv: &mut dyn RpcEnvironment) -> Result<PluginConfig, Error> {
    use crate::config::acme::plugin;

    let (plugins, digest) = plugin::config()?;
    rpcenv["digest"] = proxmox::tools::digest_to_hex(&digest).into();

    match plugins.get(&id) {
        Some((ty, data)) => Ok(modify_cfg_for_api(&id, &ty, &data)),
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
                type: DnsPluginCoreUpdater,
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
pub fn add_plugin(r#type: String, core: DnsPluginCoreUpdater, data: String) -> Result<(), Error> {
    use crate::config::acme::plugin;

    // Currently we only support DNS plugins and the standalone plugin is "fixed":
    if r#type != "dns" {
        bail!("invalid ACME plugin type: {:?}", r#type);
    }

    let data = String::from_utf8(base64::decode(&data)?)
        .map_err(|_| format_err!("data must be valid UTF-8"))?;
    //core.api_fixup()?;

    // FIXME: Solve the Updater with non-optional fields thing...
    let id = core
        .id
        .clone()
        .ok_or_else(|| format_err!("missing required 'id' parameter"))?;

    let _lock = plugin::lock()?;

    let (mut plugins, _digest) = plugin::config()?;
    if plugins.contains_key(&id) {
        bail!("ACME plugin ID {:?} already exists", id);
    }

    let plugin = serde_json::to_value(DnsPlugin {
        core: DnsPluginCore::try_build_from(core)?,
        data,
    })?;

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
    use crate::config::acme::plugin;

    let _lock = plugin::lock()?;

    let (mut plugins, _digest) = plugin::config()?;
    if plugins.remove(&id).is_none() {
        http_bail!(NOT_FOUND, "no such plugin");
    }
    plugin::save_config(&plugins)?;

    Ok(())
}

#[api(
    input: {
        properties: {
            core_update: {
                type: DnsPluginCoreUpdater,
                flatten: true,
            },
            data: {
                type: String,
                optional: true,
                // This is different in the API!
                description: "DNS plugin data (base64 encoded with padding).",
            },
            digest: {
                description: "Digest to protect against concurrent updates",
                optional: true,
            },
            delete: {
                description: "Options to remove from the configuration",
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
    core_update: DnsPluginCoreUpdater,
    data: Option<String>,
    delete: Option<String>,
    digest: Option<String>,
) -> Result<(), Error> {
    use crate::config::acme::plugin;

    let data = data
        .as_deref()
        .map(base64::decode)
        .transpose()?
        .map(String::from_utf8)
        .transpose()
        .map_err(|_| format_err!("data must be valid UTF-8"))?;
    //core_update.api_fixup()?;

    // unwrap: the id is matched by this method's API path
    let id = core_update.id.clone().unwrap();

    let delete: Vec<&str> = delete
        .as_deref()
        .unwrap_or("")
        .split(&[' ', ',', ';', '\0'][..])
        .collect();

    let _lock = plugin::lock()?;

    let (mut plugins, expected_digest) = plugin::config()?;

    if let Some(digest) = digest {
        let digest = proxmox::tools::hex_to_digest(&digest)?;
        crate::tools::detect_modified_configuration_file(&digest, &expected_digest)?;
    }

    match plugins.get_mut(&id) {
        Some((ty, ref mut entry)) => {
            if ty != "dns" {
                bail!("cannot update plugin of type {:?}", ty);
            }

            let mut plugin: DnsPlugin = serde_json::from_value(entry.clone())?;
            plugin.core.update_from(core_update, &delete)?;
            if let Some(data) = data {
                plugin.data = data;
            }
            *entry = serde_json::to_value(plugin)?;
        }
        None => http_bail!(NOT_FOUND, "no such plugin"),
    }

    plugin::save_config(&plugins)?;

    Ok(())
}
