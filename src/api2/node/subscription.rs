use anyhow::{Error, format_err, bail};
use serde_json::Value;

use proxmox::api::{api, Router, RpcEnvironment, Permission};

use crate::tools;
use crate::tools::subscription::{self, SubscriptionStatus, SubscriptionInfo};
use crate::config::acl::{PRIV_SYS_AUDIT,PRIV_SYS_MODIFY};
use crate::config::cached_user_info::CachedUserInfo;
use crate::api2::types::{NODE_SCHEMA, SUBSCRIPTION_KEY_SCHEMA, Authid};

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
pub fn check_subscription(
    force: bool,
) -> Result<(), Error> {
    let info = match subscription::read_subscription() {
        Err(err) => bail!("could not read subscription status: {}", err),
        Ok(Some(info)) => info,
        Ok(None) => return Ok(()),
    };

    let server_id = tools::get_hardware_address()?;
    let key = if let Some(key) = info.key {
        // always update apt auth if we have a key to ensure user can access enterprise repo
        subscription::update_apt_auth(Some(key.to_owned()), Some(server_id.to_owned()))?;
        key
    } else {
        String::new()
    };

    if !force && info.status == SubscriptionStatus::ACTIVE {
        let age = proxmox::tools::time::epoch_i64() - info.checktime.unwrap_or(i64::MAX);
        if age < subscription::MAX_LOCAL_KEY_AGE {
            return Ok(());
        }
    }

    let info = subscription::check_subscription(key, server_id)?;

    subscription::write_subscription(info)
        .map_err(|e| format_err!("Error writing updated subscription status - {}", e))?;

    Ok(())
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
    let url = "https://www.proxmox.com/en/proxmox-backup-server/pricing";

    let info = match subscription::read_subscription() {
        Err(err) => bail!("could not read subscription status: {}", err),
        Ok(Some(info)) => info,
        Ok(None) => SubscriptionInfo {
            status: SubscriptionStatus::NOTFOUND,
            message: Some("There is no subscription key".into()),
            serverid: Some(tools::get_hardware_address()?),
            url:  Some(url.into()),
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
pub fn set_subscription(
    key: String,
) -> Result<(), Error> {

    let server_id = tools::get_hardware_address()?;

    let info = subscription::check_subscription(key, server_id)?;

    subscription::write_subscription(info)
        .map_err(|e| format_err!("Error writing subscription status - {}", e))?;

    Ok(())
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

    subscription::delete_subscription()
        .map_err(|err| format_err!("Deleting subscription failed: {}", err))?;

    Ok(())
}

pub const ROUTER: Router = Router::new()
    .post(&API_METHOD_CHECK_SUBSCRIPTION)
    .put(&API_METHOD_SET_SUBSCRIPTION)
    .delete(&API_METHOD_DELETE_SUBSCRIPTION)
    .get(&API_METHOD_GET_SUBSCRIPTION);
