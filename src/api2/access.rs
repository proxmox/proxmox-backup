use anyhow::{bail, format_err, Error};

use serde_json::{json, Value};

use proxmox::api::{api, RpcEnvironment, Permission, UserInformation};
use proxmox::api::router::{Router, SubdirMap};
use proxmox::{sortable, identity};
use proxmox::{http_err, list_subdirs_api_method};

use crate::tools;
use crate::tools::ticket::*;
use crate::auth_helpers::*;
use crate::api2::types::*;

use crate::config::cached_user_info::CachedUserInfo;
use crate::config::acl::{PRIVILEGES, PRIV_PERMISSIONS_MODIFY};

pub mod user;
pub mod domain;
pub mod acl;
pub mod role;

/// returns Ok(true) if a ticket has to be created
/// and Ok(false) if not
fn authenticate_user(
    username: &str,
    password: &str,
    path: Option<String>,
    privs: Option<String>,
    port: Option<u16>,
) -> Result<bool, Error> {
    let user_info = CachedUserInfo::new()?;

    if !user_info.is_active_user(&username) {
        bail!("user account disabled or expired.");
    }

    let ticket_lifetime = tools::ticket::TICKET_LIFETIME;

    if password.starts_with("PBS:") {
        if let Ok((_age, Some(ticket_username))) = tools::ticket::verify_rsa_ticket(public_auth_key(), "PBS", password, None, -300, ticket_lifetime) {
            if ticket_username == username {
                return Ok(true);
            } else {
                bail!("ticket login failed - wrong username");
            }
        }
    } else if password.starts_with("PBSTERM:") {
        if path.is_none() || privs.is_none() || port.is_none() {
            bail!("cannot check termnal ticket without path, priv and port");
        }

        let path = path.unwrap();
        let privilege_name = privs.unwrap();
        let port = port.unwrap();

        if let Ok((_age, _data)) =
            tools::ticket::verify_term_ticket(public_auth_key(), &username, &path, port, password)
        {
            for (name, privilege) in PRIVILEGES {
                if *name == privilege_name {
                    let mut path_vec = Vec::new();
                    for part in path.split('/') {
                        if part != "" {
                            path_vec.push(part);
                        }
                    }

                    user_info.check_privs(username, &path_vec, *privilege, false)?;
                    return Ok(false);
                }
            }

            bail!("No such privilege");
        }
    }

    let _ = crate::auth::authenticate_user(username, password)?;
    Ok(true)
}

#[api(
    input: {
        properties: {
            username: {
                schema: PROXMOX_USER_ID_SCHEMA,
            },
            password: {
                schema: PASSWORD_SCHEMA,
            },
            path: {
                type: String,
                description: "Path for verifying terminal tickets.",
                optional: true,
            },
            privs: {
                type: String,
                description: "Privilege for verifying terminal tickets.",
                optional: true,
            },
            port: {
                type: Integer,
                description: "Port for verifying terminal tickets.",
                optional: true,
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
/// Create or verify authentication ticket.
///
/// Returns: An authentication ticket with additional infos.
fn create_ticket(
    username: String,
    password: String,
    path: Option<String>,
    privs: Option<String>,
    port: Option<u16>,
) -> Result<Value, Error> {
    match authenticate_user(&username, &password, path, privs, port) {
        Ok(true) => {
            let ticket = assemble_rsa_ticket(private_auth_key(), "PBS", Some(&username), None)?;

            let token = assemble_csrf_prevention_token(csrf_secret(), &username);

            log::info!("successful auth for user '{}'", username);

            Ok(json!({
                "username": username,
                "ticket": ticket,
                "CSRFPreventionToken": token,
            }))
        }
        Ok(false) => Ok(json!({
            "username": username,
        })),
        Err(err) => {
            let client_ip = "unknown"; // $rpcenv->get_client_ip() || '';
            log::error!("authentication failure; rhost={} user={} msg={}", client_ip, username, err.to_string());
            Err(http_err!(UNAUTHORIZED, "permission check failed."))
        }
    }
}

#[api(
    input: {
        properties: {
            userid: {
                schema: PROXMOX_USER_ID_SCHEMA,
            },
            password: {
                schema: PASSWORD_SCHEMA,
            },
        },
    },
    access: {
        description: "Anybody is allowed to change there own password. In addition, users with 'Permissions:Modify' privilege may change any password.",
        permission: &Permission::Anybody,
    },

)]
/// Change user password
///
/// Each user is allowed to change his own password. Superuser
/// can change all passwords.
fn change_password(
    userid: String,
    password: String,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let current_user = rpcenv.get_user()
        .ok_or_else(|| format_err!("unknown user"))?;

    let mut allowed = userid == current_user;

    if userid == "root@pam" { allowed = true; }

    if !allowed {
        let user_info = CachedUserInfo::new()?;
        let privs = user_info.lookup_privs(&current_user, &[]);
        if (privs & PRIV_PERMISSIONS_MODIFY) != 0 { allowed = true; }
    }

    if !allowed {
        bail!("you are not authorized to change the password.");
    }

    let (username, realm) = crate::auth::parse_userid(&userid)?;
    let authenticator = crate::auth::lookup_authenticator(&realm)?;
    authenticator.store_password(&username, &password)?;

    Ok(Value::Null)
}

#[sortable]
const SUBDIRS: SubdirMap = &sorted!([
    ("acl", &acl::ROUTER),
    (
        "password", &Router::new()
            .put(&API_METHOD_CHANGE_PASSWORD)
    ),
    (
        "ticket", &Router::new()
            .post(&API_METHOD_CREATE_TICKET)
    ),
    ("domains", &domain::ROUTER),
    ("roles", &role::ROUTER),
    ("users", &user::ROUTER),
]);

pub const ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(SUBDIRS))
    .subdirs(SUBDIRS);
