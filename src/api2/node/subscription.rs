use anyhow::{bail, format_err, Error};
use serde_json::Value;

use proxmox_http::client::Client;
use proxmox_http::HttpOptions;
use proxmox_router::{Permission, Router, RpcEnvironment};
use proxmox_schema::api;
use proxmox_subscription::{SubscriptionInfo, SubscriptionStatus};
use proxmox_sys::fs::CreateOptions;

use pbs_api_types::{
    Authid, NODE_SCHEMA, PRIV_SYS_AUDIT, PRIV_SYS_MODIFY, SUBSCRIPTION_KEY_SCHEMA,
};

use crate::config::node;
use crate::tools::{DEFAULT_USER_AGENT_STRING, PROXMOX_BACKUP_TCP_KEEPALIVE_TIME};

use pbs_buildcfg::PROXMOX_BACKUP_SUBSCRIPTION_FN;
use pbs_config::CachedUserInfo;

const PRODUCT_URL: &str = "https://www.proxmox.com/en/proxmox-backup-server/pricing";
const APT_AUTH_FN: &str = "/etc/apt/auth.conf.d/pbs.conf";
const APT_AUTH_URL: &str = "enterprise.proxmox.com/debian/pbs";

pub fn subscription_file_opts() -> Result<CreateOptions, Error> {
    let backup_user = pbs_config::backup_user()?;
    let mode = nix::sys::stat::Mode::from_bits_truncate(0o0640);
    Ok(CreateOptions::new()
        .perm(mode)
        .owner(nix::unistd::ROOT)
        .group(backup_user.gid))
}

fn apt_auth_file_opts() -> CreateOptions {
    let mode = nix::sys::stat::Mode::from_bits_truncate(0o0600);
    CreateOptions::new().perm(mode).owner(nix::unistd::ROOT)
}

fn check_and_write_subscription(key: String, server_id: String) -> Result<(), Error> {
    let proxy_config = if let Ok((node_config, _digest)) = node::config() {
        node_config.http_proxy()
    } else {
        None
    };

    let client = Client::with_options(HttpOptions {
        proxy_config,
        user_agent: Some(DEFAULT_USER_AGENT_STRING.to_string()),
        tcp_keepalive: Some(PROXMOX_BACKUP_TCP_KEEPALIVE_TIME),
    });

    let info = proxmox_subscription::check::check_subscription(
        key,
        server_id,
        PRODUCT_URL.to_string(),
        client,
    )?;

    proxmox_subscription::files::write_subscription(
        PROXMOX_BACKUP_SUBSCRIPTION_FN,
        subscription_file_opts()?,
        &info,
    )
    .map_err(|e| format_err!("Error writing updated subscription status - {}", e))?;

    proxmox_subscription::files::update_apt_auth(
        APT_AUTH_FN,
        apt_auth_file_opts(),
        APT_AUTH_URL,
        info.key,
        info.serverid,
    )
}

#[api(
    input: {
        properties: {
            node: {
                schema: NODE_SCHEMA,
            },
            force: {
                description: "Always connect to server, even if information in cache is up to date.",
                type: bool,
                optional: true,
                default: false,
            },
        },
    },
    protected: true,
    access: {
        permission: &Permission::Privilege(&["system"], PRIV_SYS_MODIFY, false),
    },
)]
/// Check and update subscription status.
pub fn check_subscription(force: bool) -> Result<(), Error> {
    let mut info = match proxmox_subscription::files::read_subscription(
        PROXMOX_BACKUP_SUBSCRIPTION_FN,
        &[proxmox_subscription::files::DEFAULT_SIGNING_KEY],
    ) {
        Err(err) => bail!("could not read subscription status: {}", err),
        Ok(Some(info)) => info,
        Ok(None) => return Ok(()),
    };

    let server_id = proxmox_subscription::get_hardware_address()?;
    let key = if let Some(key) = info.key.as_ref() {
        // always update apt auth if we have a key to ensure user can access enterprise repo
        proxmox_subscription::files::update_apt_auth(
            APT_AUTH_FN,
            apt_auth_file_opts(),
            APT_AUTH_URL,
            Some(key.to_owned()),
            Some(server_id.to_owned()),
        )?;
        key.to_owned()
    } else {
        String::new()
    };

    if info.is_signed() {
        bail!("Updating offline key not possible - please remove and re-add subscription key to switch to online key.");
    }

    if !force && info.status == SubscriptionStatus::Active {
        // will set to INVALID if last check too long ago
        info.check_age(true);
        if info.status == SubscriptionStatus::Active {
            return Ok(());
        }
    }

    check_and_write_subscription(key, server_id)
}

#[api(
    input: {
        properties: {
            node: {
                schema: NODE_SCHEMA,
            },
        },
    },
    returns: { type: SubscriptionInfo },
    access: {
        permission: &Permission::Anybody,
    },
)]
/// Read subscription info.
pub fn get_subscription(
    _param: Value,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<SubscriptionInfo, Error> {
    let info = match proxmox_subscription::files::read_subscription(
        PROXMOX_BACKUP_SUBSCRIPTION_FN,
        &[proxmox_subscription::files::DEFAULT_SIGNING_KEY],
    ) {
        Err(err) => bail!("could not read subscription status: {}", err),
        Ok(Some(info)) => info,
        Ok(None) => SubscriptionInfo {
            status: SubscriptionStatus::NotFound,
            message: Some("There is no subscription key".into()),
            serverid: Some(proxmox_subscription::get_hardware_address()?),
            url: Some(PRODUCT_URL.into()),
            ..Default::default()
        },
    };

    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let user_info = CachedUserInfo::new()?;
    let user_privs = user_info.lookup_privs(&auth_id, &[]);

    if (user_privs & PRIV_SYS_AUDIT) == 0 {
        // not enough privileges for full state
        return Ok(SubscriptionInfo {
            status: info.status,
            message: info.message,
            url: info.url,
            ..Default::default()
        });
    };

    Ok(info)
}

#[api(
    input: {
        properties: {
            node: {
                schema: NODE_SCHEMA,
            },
            key: {
                schema: SUBSCRIPTION_KEY_SCHEMA,
            },
        },
    },
    protected: true,
    access: {
        permission: &Permission::Privilege(&["system"], PRIV_SYS_MODIFY, false),
    },
)]
/// Set a subscription key and check it.
pub fn set_subscription(key: String) -> Result<(), Error> {
    let server_id = proxmox_subscription::get_hardware_address()?;

    check_and_write_subscription(key, server_id)
}

#[api(
    input: {
        properties: {
            node: {
                schema: NODE_SCHEMA,
            },
        },
    },
    protected: true,
    access: {
        permission: &Permission::Privilege(&["system"], PRIV_SYS_MODIFY, false),
    },
)]
/// Delete subscription info.
pub fn delete_subscription() -> Result<(), Error> {
    proxmox_subscription::files::delete_subscription(PROXMOX_BACKUP_SUBSCRIPTION_FN)
        .map_err(|err| format_err!("Deleting subscription failed: {}", err))?;

    proxmox_subscription::files::update_apt_auth(
        APT_AUTH_FN,
        apt_auth_file_opts(),
        APT_AUTH_URL,
        None,
        None,
    )?;

    Ok(())
}

pub const ROUTER: Router = Router::new()
    .post(&API_METHOD_CHECK_SUBSCRIPTION)
    .put(&API_METHOD_SET_SUBSCRIPTION)
    .delete(&API_METHOD_DELETE_SUBSCRIPTION)
    .get(&API_METHOD_GET_SUBSCRIPTION);
