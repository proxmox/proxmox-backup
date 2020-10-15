use anyhow::{bail, format_err, Error};

use serde_json::{json, Value};

use proxmox::api::{api, RpcEnvironment, Permission};
use proxmox::api::router::{Router, SubdirMap};
use proxmox::{sortable, identity};
use proxmox::{http_err, list_subdirs_api_method};

use crate::tools::ticket::{self, Empty, Ticket};
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
    userid: &Userid,
    password: &str,
    path: Option<String>,
    privs: Option<String>,
    port: Option<u16>,
) -> Result<bool, Error> {
    let user_info = CachedUserInfo::new()?;

    if !user_info.is_active_user(&userid) {
        bail!("user account disabled or expired.");
    }

    if password.starts_with("PBS:") {
        if let Ok(ticket_userid) = Ticket::<Userid>::parse(password)
            .and_then(|ticket| ticket.verify(public_auth_key(), "PBS", None))
        {
            if *userid == ticket_userid {
                return Ok(true);
            }
            bail!("ticket login failed - wrong userid");
        }
    } else if password.starts_with("PBSTERM:") {
        if path.is_none() || privs.is_none() || port.is_none() {
            bail!("cannot check termnal ticket without path, priv and port");
        }

        let path = path.ok_or_else(|| format_err!("missing path for termproxy ticket"))?;
        let privilege_name = privs
            .ok_or_else(|| format_err!("missing privilege name for termproxy ticket"))?;
        let port = port.ok_or_else(|| format_err!("missing port for termproxy ticket"))?;

        if let Ok(Empty) = Ticket::parse(password)
            .and_then(|ticket| ticket.verify(
                public_auth_key(),
                ticket::TERM_PREFIX,
                Some(&ticket::term_aad(userid, &path, port)),
            ))
        {
            for (name, privilege) in PRIVILEGES {
                if *name == privilege_name {
                    let mut path_vec = Vec::new();
                    for part in path.split('/') {
                        if part != "" {
                            path_vec.push(part);
                        }
                    }

                    user_info.check_privs(userid, &path_vec, *privilege, false)?;
                    return Ok(false);
                }
            }

            bail!("No such privilege");
        }
    }

    let _ = crate::auth::authenticate_user(userid, password)?;
    Ok(true)
}

#[api(
    input: {
        properties: {
            username: {
                type: Userid,
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
    username: Userid,
    password: String,
    path: Option<String>,
    privs: Option<String>,
    port: Option<u16>,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {
    match authenticate_user(&username, &password, path, privs, port) {
        Ok(true) => {
            let ticket = Ticket::new("PBS", &username)?.sign(private_auth_key(), None)?;

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
            let client_ip = match rpcenv.get_client_ip().map(|addr| addr.ip()) {
                Some(ip) => format!("{}", ip),
                None => "unknown".into(),
            };

            log::error!("authentication failure; rhost={} user={} msg={}", client_ip, username, err.to_string());
            Err(http_err!(UNAUTHORIZED, "permission check failed."))
        }
    }
}

#[api(
    input: {
        properties: {
            userid: {
                type: Userid,
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
    userid: Userid,
    password: String,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let current_user: Userid = rpcenv
        .get_user()
        .ok_or_else(|| format_err!("unknown user"))?
        .parse()?;

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

    let authenticator = crate::auth::lookup_authenticator(userid.realm())?;
    authenticator.store_password(userid.name(), &password)?;

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
