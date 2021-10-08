//! Access control (Users, Permissions and Authentication)

use anyhow::{bail, format_err, Error};

use serde_json::{json, Value};
use std::collections::HashMap;
use std::collections::HashSet;

use proxmox::{identity, sortable};
use proxmox_router::{
    http_err, list_subdirs_api_method, Router, RpcEnvironment, SubdirMap, Permission,
};
use proxmox_schema::api;

use pbs_api_types::{
    Userid, Authid, PASSWORD_SCHEMA, ACL_PATH_SCHEMA,
    PRIVILEGES, PRIV_PERMISSIONS_MODIFY, PRIV_SYS_AUDIT,
};
use pbs_tools::ticket::{self, Empty, Ticket};
use pbs_config::acl::AclTreeNode;
use pbs_config::CachedUserInfo;

use crate::auth_helpers::*;
use crate::config::tfa::TfaChallenge;
use crate::server::ticket::ApiTicket;

pub mod acl;
pub mod domain;
pub mod openid;
pub mod role;
pub mod tfa;
pub mod user;

#[allow(clippy::large_enum_variant)]
enum AuthResult {
    /// Successful authentication which does not require a new ticket.
    Success,

    /// Successful authentication which requires a ticket to be created.
    CreateTicket,

    /// A partial ticket which requires a 2nd factor will be created.
    Partial(TfaChallenge),
}

fn authenticate_user(
    userid: &Userid,
    password: &str,
    path: Option<String>,
    privs: Option<String>,
    port: Option<u16>,
    tfa_challenge: Option<String>,
) -> Result<AuthResult, Error> {
    let user_info = CachedUserInfo::new()?;

    let auth_id = Authid::from(userid.clone());
    if !user_info.is_active_auth_id(&auth_id) {
        bail!("user account disabled or expired.");
    }

    if let Some(tfa_challenge) = tfa_challenge {
        return authenticate_2nd(userid, &tfa_challenge, password);
    }

    if password.starts_with("PBS:") {
        if let Ok(ticket_userid) = Ticket::<Userid>::parse(password)
            .and_then(|ticket| ticket.verify(public_auth_key(), "PBS", None))
        {
            if *userid == ticket_userid {
                return Ok(AuthResult::CreateTicket);
            }
            bail!("ticket login failed - wrong userid");
        }
    } else if password.starts_with("PBSTERM:") {
        if path.is_none() || privs.is_none() || port.is_none() {
            bail!("cannot check termnal ticket without path, priv and port");
        }

        let path = path.ok_or_else(|| format_err!("missing path for termproxy ticket"))?;
        let privilege_name =
            privs.ok_or_else(|| format_err!("missing privilege name for termproxy ticket"))?;
        let port = port.ok_or_else(|| format_err!("missing port for termproxy ticket"))?;

        if let Ok(Empty) = Ticket::parse(password).and_then(|ticket| {
            ticket.verify(
                public_auth_key(),
                ticket::TERM_PREFIX,
                Some(&crate::tools::ticket::term_aad(userid, &path, port)),
            )
        }) {
            for (name, privilege) in PRIVILEGES {
                if *name == privilege_name {
                    let mut path_vec = Vec::new();
                    for part in path.split('/') {
                        if part != "" {
                            path_vec.push(part);
                        }
                    }
                    user_info.check_privs(&auth_id, &path_vec, *privilege, false)?;
                    return Ok(AuthResult::Success);
                }
            }

            bail!("No such privilege");
        }
    }

    let _: () = crate::auth::authenticate_user(userid, password)?;

    Ok(match crate::config::tfa::login_challenge(userid)? {
        None => AuthResult::CreateTicket,
        Some(challenge) => AuthResult::Partial(challenge),
    })
}

fn authenticate_2nd(
    userid: &Userid,
    challenge_ticket: &str,
    response: &str,
) -> Result<AuthResult, Error> {
    let challenge: TfaChallenge = Ticket::<ApiTicket>::parse(&challenge_ticket)?
        .verify_with_time_frame(public_auth_key(), "PBS", Some(userid.as_str()), -60..600)?
        .require_partial()?;

    let _: () = crate::config::tfa::verify_challenge(userid, &challenge, response.parse()?)?;

    Ok(AuthResult::CreateTicket)
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
            "tfa-challenge": {
                type: String,
                description: "The signed TFA challenge string the user wants to respond to.",
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
                description:
                    "Cross Site Request Forgery Prevention Token. \
                     For partial tickets this is the string \"invalid\".",
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
pub fn create_ticket(
    username: Userid,
    password: String,
    path: Option<String>,
    privs: Option<String>,
    port: Option<u16>,
    tfa_challenge: Option<String>,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    use proxmox_rest_server::RestEnvironment;

    let env: &RestEnvironment = rpcenv.as_any().downcast_ref::<RestEnvironment>()
        .ok_or_else(|| format_err!("detected worng RpcEnvironment type"))?;

    match authenticate_user(&username, &password, path, privs, port, tfa_challenge) {
        Ok(AuthResult::Success) => Ok(json!({ "username": username })),
        Ok(AuthResult::CreateTicket) => {
            let api_ticket = ApiTicket::full(username.clone());
            let ticket = Ticket::new("PBS", &api_ticket)?.sign(private_auth_key(), None)?;
            let token = assemble_csrf_prevention_token(csrf_secret(), &username);

            env.log_auth(username.as_str());

            Ok(json!({
                "username": username,
                "ticket": ticket,
                "CSRFPreventionToken": token,
            }))
        }
        Ok(AuthResult::Partial(challenge)) => {
            let api_ticket = ApiTicket::partial(challenge);
            let ticket = Ticket::new("PBS", &api_ticket)?
                .sign(private_auth_key(), Some(username.as_str()))?;
            Ok(json!({
                "username": username,
                "ticket": ticket,
                "CSRFPreventionToken": "invalid",
            }))
        }
        Err(err) => {
            env.log_failed_auth(Some(username.to_string()), &err.to_string());
            Err(http_err!(UNAUTHORIZED, "permission check failed."))
        }
    }
}

#[api(
    protected: true,
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
        description: "Everybody is allowed to change their own password. In addition, users with 'Permissions:Modify' privilege may change any password on @pbs realm.",
        permission: &Permission::Anybody,
    },
)]
/// Change user password
///
/// Each user is allowed to change his own password. Superuser
/// can change all passwords.
pub fn change_password(
    userid: Userid,
    password: String,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {
    let current_auth: Authid = rpcenv
        .get_auth_id()
        .ok_or_else(|| format_err!("no authid available"))?
        .parse()?;

    if current_auth.is_token() {
        bail!("API tokens cannot access this API endpoint");
    }

    let current_user = current_auth.user();

    let mut allowed = userid == *current_user;

    if !allowed {
        let user_info = CachedUserInfo::new()?;
        let privs = user_info.lookup_privs(&current_auth, &[]);
        if user_info.is_superuser(&current_auth) {
            allowed = true;
        }
        if (privs & PRIV_PERMISSIONS_MODIFY) != 0 && userid.realm() != "pam" {
            allowed = true;
        }
    };

    if !allowed {
        bail!("you are not authorized to change the password.");
    }

    let authenticator = crate::auth::lookup_authenticator(userid.realm())?;
    authenticator.store_password(userid.name(), &password)?;

    Ok(Value::Null)
}

#[api(
    input: {
        properties: {
            "auth-id": {
                type: Authid,
                optional: true,
            },
            path: {
                schema: ACL_PATH_SCHEMA,
                optional: true,
            },
        },
    },
    access: {
        permission: &Permission::Anybody,
        description: "Requires Sys.Audit on '/access', limited to own privileges otherwise.",
    },
    returns: {
        description: "Map of ACL path to Map of privilege to propagate bit",
        type: Object,
        properties: {},
        additional_properties: true,
    },
)]
/// List permissions of given or currently authenticated user / API token.
///
/// Optionally limited to specific path.
pub fn list_permissions(
    auth_id: Option<Authid>,
    path: Option<String>,
    rpcenv: &dyn RpcEnvironment,
) -> Result<HashMap<String, HashMap<String, bool>>, Error> {
    let current_auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;

    let user_info = CachedUserInfo::new()?;
    let user_privs = user_info.lookup_privs(&current_auth_id, &["access"]);

    let auth_id = match auth_id {
        Some(auth_id) if auth_id == current_auth_id => current_auth_id,
        Some(auth_id) => {
            if user_privs & PRIV_SYS_AUDIT != 0
                || (auth_id.is_token()
                    && !current_auth_id.is_token()
                    && auth_id.user() == current_auth_id.user())
            {
                auth_id
            } else {
                bail!("not allowed to list permissions of {}", auth_id);
            }
        },
        None => current_auth_id,
    };

    fn populate_acl_paths(
        mut paths: HashSet<String>,
        node: AclTreeNode,
        path: &str,
    ) -> HashSet<String> {
        for (sub_path, child_node) in node.children {
            let sub_path = format!("{}/{}", path, &sub_path);
            paths = populate_acl_paths(paths, child_node, &sub_path);
            paths.insert(sub_path);
        }
        paths
    }

    let paths = match path {
        Some(path) => {
            let mut paths = HashSet::new();
            paths.insert(path);
            paths
        }
        None => {
            let mut paths = HashSet::new();

            let (acl_tree, _) = pbs_config::acl::config()?;
            paths = populate_acl_paths(paths, acl_tree.root, "");

            // default paths, returned even if no ACL exists
            paths.insert("/".to_string());
            paths.insert("/access".to_string());
            paths.insert("/datastore".to_string());
            paths.insert("/remote".to_string());
            paths.insert("/system".to_string());

            paths
        }
    };

    let map = paths.into_iter().fold(
        HashMap::new(),
        |mut map: HashMap<String, HashMap<String, bool>>, path: String| {
            let split_path = pbs_config::acl::split_acl_path(path.as_str());
            let (privs, propagated_privs) = user_info.lookup_privs_details(&auth_id, &split_path);

            match privs {
                0 => map, // Don't leak ACL paths where we don't have any privileges
                _ => {
                    let priv_map =
                        PRIVILEGES
                            .iter()
                            .fold(HashMap::new(), |mut priv_map, (name, value)| {
                                if value & privs != 0 {
                                    priv_map
                                        .insert(name.to_string(), value & propagated_privs != 0);
                                }
                                priv_map
                            });

                    map.insert(path, priv_map);
                    map
                }
            }
        },
    );

    Ok(map)
}

#[sortable]
const SUBDIRS: SubdirMap = &sorted!([
    ("acl", &acl::ROUTER),
    ("password", &Router::new().put(&API_METHOD_CHANGE_PASSWORD)),
    (
        "permissions",
        &Router::new().get(&API_METHOD_LIST_PERMISSIONS)
    ),
    ("ticket", &Router::new().post(&API_METHOD_CREATE_TICKET)),
    ("openid", &openid::ROUTER),
    ("domains", &domain::ROUTER),
    ("roles", &role::ROUTER),
    ("users", &user::ROUTER),
    ("tfa", &tfa::ROUTER),
]);

pub const ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(SUBDIRS))
    .subdirs(SUBDIRS);
