use anyhow::Error;

use serde_json::{json, Value};

use proxmox::api::{api, Permission};
use proxmox::api::router::Router;

use crate::api2::types::*;
use crate::config::acl::{Role, ROLE_NAMES};

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

    for (role, comment) in ROLE_NAMES.iter() {
        list.push(json!({ "role": role, "comment": comment }));
    }
    Ok(list.into())
}

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_LIST_ROLES);
