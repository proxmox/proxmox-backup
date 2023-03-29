//! OpenID redirect/login API
use anyhow::{bail, format_err, Error};
use serde_json::{json, Value};

use proxmox_auth_api::api::ApiTicket;
use proxmox_auth_api::ticket::Ticket;
use proxmox_router::{
    http_err, list_subdirs_api_method, Permission, Router, RpcEnvironment, SubdirMap,
};
use proxmox_schema::api;
use proxmox_sortable_macro::sortable;

use proxmox_openid::{OpenIdAuthenticator, OpenIdConfig};

use pbs_api_types::{
    OpenIdRealmConfig, User, Userid, EMAIL_SCHEMA, FIRST_NAME_SCHEMA, LAST_NAME_SCHEMA,
    OPENID_DEFAILT_SCOPE_LIST, REALM_ID_SCHEMA,
};
use pbs_buildcfg::PROXMOX_BACKUP_RUN_DIR_M;

use pbs_config::open_backup_lockfile;
use pbs_config::CachedUserInfo;

use crate::auth::private_auth_keyring;
use crate::auth_helpers::*;

fn openid_authenticator(
    realm_config: &OpenIdRealmConfig,
    redirect_url: &str,
) -> Result<OpenIdAuthenticator, Error> {
    let scopes: Vec<String> = realm_config
        .scopes
        .as_deref()
        .unwrap_or(OPENID_DEFAILT_SCOPE_LIST)
        .split(|c: char| c == ',' || c == ';' || char::is_ascii_whitespace(&c))
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect();

    let mut acr_values = None;
    if let Some(ref list) = realm_config.acr_values {
        acr_values = Some(
            list.split(|c: char| c == ',' || c == ';' || char::is_ascii_whitespace(&c))
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect(),
        );
    }

    let config = OpenIdConfig {
        issuer_url: realm_config.issuer_url.clone(),
        client_id: realm_config.client_id.clone(),
        client_key: realm_config.client_key.clone(),
        prompt: realm_config.prompt.clone(),
        scopes: Some(scopes),
        acr_values,
    };
    OpenIdAuthenticator::discover(&config, redirect_url)
}

#[api(
    input: {
        properties: {
            state: {
                description: "OpenId state.",
                type: String,
            },
            code: {
                description: "OpenId authorization code.",
                type: String,
            },
            "redirect-url": {
                description: "Redirection Url. The client should set this to used server url.",
                type: String,
            },
        },
    },
    returns: {
        properties: {
            username: {
                type: String,
                description: "User name.",
            },
            ticket: {
                type: String,
                description: "Auth ticket.",
            },
            CSRFPreventionToken: {
                type: String,
                description: "Cross Site Request Forgery Prevention Token.",
            },
        },
    },
    protected: true,
    access: {
        permission: &Permission::World,
    },
)]
/// Verify OpenID authorization code and create a ticket
///
/// Returns: An authentication ticket with additional infos.
pub fn openid_login(
    state: String,
    code: String,
    redirect_url: String,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {
    use proxmox_rest_server::RestEnvironment;

    let env: &RestEnvironment = rpcenv
        .as_any()
        .downcast_ref::<RestEnvironment>()
        .ok_or_else(|| format_err!("detected wrong RpcEnvironment type"))?;

    let user_info = CachedUserInfo::new()?;

    let mut tested_username = None;

    let result = proxmox_lang::try_block!({
        let (realm, private_auth_state) =
            OpenIdAuthenticator::verify_public_auth_state(PROXMOX_BACKUP_RUN_DIR_M!(), &state)?;

        let (domains, _digest) = pbs_config::domains::config()?;
        let config: OpenIdRealmConfig = domains.lookup("openid", &realm)?;

        let open_id = openid_authenticator(&config, &redirect_url)?;

        let info = open_id.verify_authorization_code_simple(&code, &private_auth_state)?;

        // eprintln!("VERIFIED {:?}", info);

        let name_attr = config.username_claim.as_deref().unwrap_or("sub");

        // Try to be compatible with previous versions
        let try_attr = match name_attr {
            "subject" => Some("sub"),
            "username" => Some("preferred_username"),
            _ => None,
        };

        let unique_name = match info[name_attr].as_str() {
            Some(name) => name.to_owned(),
            None => {
                if let Some(try_attr) = try_attr {
                    match info[try_attr].as_str() {
                        Some(name) => name.to_owned(),
                        None => bail!("missing claim '{}'", name_attr),
                    }
                } else {
                    bail!("missing claim '{}'", name_attr);
                }
            }
        };

        let user_id = Userid::try_from(format!("{}@{}", unique_name, realm))?;
        tested_username = Some(unique_name);

        if !user_info.is_active_user_id(&user_id) {
            if config.autocreate.unwrap_or(false) {
                use pbs_config::user;
                let _lock = open_backup_lockfile(user::USER_CFG_LOCKFILE, None, true)?;

                let firstname = info["given_name"]
                    .as_str()
                    .map(|n| n.to_string())
                    .filter(|n| FIRST_NAME_SCHEMA.parse_simple_value(n).is_ok());

                let lastname = info["family_name"]
                    .as_str()
                    .map(|n| n.to_string())
                    .filter(|n| LAST_NAME_SCHEMA.parse_simple_value(n).is_ok());

                let email = info["email"]
                    .as_str()
                    .map(|n| n.to_string())
                    .filter(|n| EMAIL_SCHEMA.parse_simple_value(n).is_ok());

                let user = User {
                    userid: user_id.clone(),
                    comment: None,
                    enable: None,
                    expire: None,
                    firstname,
                    lastname,
                    email,
                };
                let (mut config, _digest) = user::config()?;
                if let Ok(old_user) = config.lookup::<User>("user", user.userid.as_str()) {
                    if let Some(false) = old_user.enable {
                        bail!("user '{}' is disabled.", user.userid);
                    } else {
                        bail!("autocreate user failed - '{}' already exists.", user.userid);
                    }
                }
                config.set_data(user.userid.as_str(), "user", &user)?;
                user::save_config(&config)?;
            } else {
                bail!("user account '{}' missing, disabled or expired.", user_id);
            }
        }

        let api_ticket = ApiTicket::Full(user_id.clone());
        let ticket = Ticket::new("PBS", &api_ticket)?.sign(private_auth_keyring(), None)?;
        let token = assemble_csrf_prevention_token(csrf_secret(), &user_id);

        env.log_auth(user_id.as_str());

        Ok(json!({
            "username": user_id,
            "ticket": ticket,
            "CSRFPreventionToken": token,
        }))
    });

    if let Err(ref err) = result {
        let msg = err.to_string();
        env.log_failed_auth(tested_username, &msg);
        return Err(http_err!(UNAUTHORIZED, "{}", msg));
    }

    result
}

#[api(
    protected: true,
    input: {
        properties: {
            realm: {
                schema: REALM_ID_SCHEMA,
            },
            "redirect-url": {
                description: "Redirection Url. The client should set this to used server url.",
                type: String,
            },
        },
    },
    returns: {
        description: "Redirection URL.",
        type: String,
    },
    access: {
        description: "Anyone can access this (before the user is authenticated).",
        permission: &Permission::World,
    },
)]
/// Create OpenID Redirect Session
fn openid_auth_url(
    realm: String,
    redirect_url: String,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<String, Error> {
    let (domains, _digest) = pbs_config::domains::config()?;
    let config: OpenIdRealmConfig = domains.lookup("openid", &realm)?;

    let open_id = openid_authenticator(&config, &redirect_url)?;

    let url = open_id.authorize_url(PROXMOX_BACKUP_RUN_DIR_M!(), &realm)?;

    Ok(url)
}

#[sortable]
const SUBDIRS: SubdirMap = &sorted!([
    ("login", &Router::new().post(&API_METHOD_OPENID_LOGIN)),
    ("auth-url", &Router::new().post(&API_METHOD_OPENID_AUTH_URL)),
]);

pub const ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(SUBDIRS))
    .subdirs(SUBDIRS);
