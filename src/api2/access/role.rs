//! Manage Roles with privileges

use anyhow::Error;

use serde_json::{json, Value};

use proxmox_router::{Permission, Router};
use proxmox_schema::api;

use pbs_api_types::{Role, PRIVILEGES, SINGLE_LINE_COMMENT_SCHEMA};
use pbs_config::acl::ROLE_NAMES;

#[api(
    returns: {
        description: "List of roles.",
        type: Array,
        items: {
            type: Object,
            description: "Role with description and privileges.",
            properties: {
                roleid: {
                    type: Role,
                },
                privs: {
                    type: Array,
                    description: "List of Privileges",
                    items: {
                        type: String,
                        description: "A Privilege",
                    },
                },
                comment: {
                    schema: SINGLE_LINE_COMMENT_SCHEMA,
                    optional: true,
                },
            },
        }
    },
    access: {
        permission: &Permission::Anybody,
    }
)]
/// Role list
fn list_roles() -> Result<Value, Error> {
    let mut list = Vec::new();

    for (role, (privs, comment)) in ROLE_NAMES.iter() {
        let mut priv_list = Vec::new();
        for (name, privilege) in PRIVILEGES.iter() {
            if privs & privilege > 0 {
                priv_list.push(name);
            }
        }
        list.push(json!({ "roleid": role, "privs": priv_list, "comment": comment }));
    }
    Ok(list.into())
}

pub const ROUTER: Router = Router::new().get(&API_METHOD_LIST_ROLES);
