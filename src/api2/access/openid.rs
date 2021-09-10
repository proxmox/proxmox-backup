//! OpenID redirect/login API
use std::convert::TryFrom;

use anyhow::{bail, Error};

use serde_json::{json, Value};

use proxmox::api::router::{Router, SubdirMap};
use proxmox::api::{api, Permission, RpcEnvironment};
use proxmox::{list_subdirs_api_method};
use proxmox::{identity, sortable};

use proxmox_openid::{OpenIdAuthenticator,  OpenIdConfig};

use pbs_api_types::{Userid, User, REALM_ID_SCHEMA};
use pbs_buildcfg::PROXMOX_BACKUP_RUN_DIR_M;
use pbs_tools::auth::private_auth_key;
use pbs_tools::ticket::Ticket;
use pbs_config::domains::{OpenIdUserAttribute, OpenIdRealmConfig};

use crate::server::ticket::ApiTicket;
use pbs_config::CachedUserInfo;

use pbs_config::open_backup_lockfile;

use crate::auth_helpers::*;

fn openid_authenticator(realm_config: &OpenIdRealmConfig, redirect_url: &str) -> Result<OpenIdAuthenticator, Error> {
    let config = OpenIdConfig {
        issuer_url: realm_config.issuer_url.clone(),
        client_id: realm_config.client_id.clone(),
        client_key: realm_config.client_key.clone(),
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
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {
    let user_info = CachedUserInfo::new()?;

    let (realm, private_auth_state) =
        OpenIdAuthenticator::verify_public_auth_state(PROXMOX_BACKUP_RUN_DIR_M!(), &state)?;

    let (domains, _digest) = pbs_config::domains::config()?;
    let config: OpenIdRealmConfig = domains.lookup("openid", &realm)?;

    let open_id = openid_authenticator(&config, &redirect_url)?;

    let info = open_id.verify_authorization_code(&code, &private_auth_state)?;

    // eprintln!("VERIFIED {} {:?} {:?}", info.subject().as_str(), info.name(), info.email());

    let unique_name = match config.username_claim {
        None | Some(OpenIdUserAttribute::Subject) => info.subject().as_str(),
        Some(OpenIdUserAttribute::Username) => {
            match info.preferred_username() {
                Some(name) => name.as_str(),
                None => bail!("missing claim 'preferred_name'"),
            }
        }
        Some(OpenIdUserAttribute::Email) => {
            match info.email() {
                Some(name) => name.as_str(),
                None => bail!("missing claim 'email'"),
            }
        }
    };

    let user_id = Userid::try_from(format!("{}@{}", unique_name, realm))?;

    if !user_info.is_active_user_id(&user_id) {
        if config.autocreate.unwrap_or(false) {
            use pbs_config::user;
            let _lock = open_backup_lockfile(user::USER_CFG_LOCKFILE, None, true)?;
            let user = User {
                userid: user_id.clone(),
                comment: None,
                enable: None,
                expire: None,
                firstname: info.given_name().and_then(|n| n.get(None)).map(|n| n.to_string()),
                lastname: info.family_name().and_then(|n| n.get(None)).map(|n| n.to_string()),
                email: info.email().map(|e| e.to_string()),
            };
            let (mut config, _digest) = user::config()?;
            if config.sections.get(user.userid.as_str()).is_some() {
                bail!("autocreate user failed - '{}' already exists.", user.userid);
            }
            config.set_data(user.userid.as_str(), "user", &user)?;
            user::save_config(&config)?;
        } else {
            bail!("user account '{}' missing, disabled or expired.", user_id);
        }
    }

    let api_ticket = ApiTicket::full(user_id.clone());
    let ticket = Ticket::new("PBS", &api_ticket)?.sign(private_auth_key(), None)?;
    let token = assemble_csrf_prevention_token(csrf_secret(), &user_id);

    crate::server::rest::auth_logger()?
        .log(format!("successful auth for user '{}'", user_id));

    Ok(json!({
        "username": user_id,
        "ticket": ticket,
        "CSRFPreventionToken": token,
    }))
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

    let url = open_id.authorize_url(PROXMOX_BACKUP_RUN_DIR_M!(), &realm)?
        .to_string();

    Ok(url.into())
}

#[sortable]
const SUBDIRS: SubdirMap = &sorted!([
    ("login", &Router::new().post(&API_METHOD_OPENID_LOGIN)),
    ("auth-url", &Router::new().post(&API_METHOD_OPENID_AUTH_URL)),
]);

pub const ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(SUBDIRS))
    .subdirs(SUBDIRS);
