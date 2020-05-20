use anyhow::Error;

use serde_json::{json, Value};

use proxmox::api::{api, Permission};
use proxmox::api::router::Router;

use crate::api2::types::*;
use crate::config::acl::{Role, ROLE_NAMES, PRIVILEGES};

#[api(
    returns: {
        description: "List of roles.",
        type: Array,
        items: {
            type: Object,
            description: "User name with description.",
            properties: {
                role: {
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
                priv_list.push(name.clone());
            }
        }
        list.push(json!({ "role": role, "privs": priv_list, "comment": comment }));
    }
    Ok(list.into())
}

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_LIST_ROLES);
