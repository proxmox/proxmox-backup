//! Access control (Users, Permissions and Authentication)

use anyhow::{bail, format_err, Error};

use serde_json::Value;
use std::collections::HashMap;
use std::collections::HashSet;

use proxmox_router::{list_subdirs_api_method, Permission, Router, RpcEnvironment, SubdirMap};
use proxmox_schema::api;
use proxmox_sortable_macro::sortable;

use pbs_api_types::{
    Authid, Userid, ACL_PATH_SCHEMA, PASSWORD_SCHEMA, PRIVILEGES, PRIV_PERMISSIONS_MODIFY,
    PRIV_SYS_AUDIT,
};
use pbs_config::acl::AclTreeNode;
use pbs_config::CachedUserInfo;

pub mod acl;
pub mod domain;
pub mod openid;
pub mod role;
pub mod tfa;
pub mod user;

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
    let client_ip = rpcenv.get_client_ip().map(|sa| sa.ip());
    authenticator.store_password(userid.name(), &password, client_ip.as_ref())?;

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
        }
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
    (
        "ticket",
        &Router::new().post(&proxmox_auth_api::api::API_METHOD_CREATE_TICKET)
    ),
    ("openid", &openid::ROUTER),
    ("domains", &domain::ROUTER),
    ("roles", &role::ROUTER),
    ("users", &user::ROUTER),
    ("tfa", &tfa::ROUTER),
]);

pub const ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(SUBDIRS))
    .subdirs(SUBDIRS);
