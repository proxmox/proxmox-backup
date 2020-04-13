use failure::*;
use serde_json::Value;
use ::serde::{Deserialize, Serialize};

use proxmox::api::{api, ApiMethod, Router, RpcEnvironment};
use proxmox::api::schema::{Schema, StringSchema, BooleanSchema, ApiStringFormat};

use crate::api2::types::*;
use crate::config::acl;

pub const ACL_PROPAGATE_SCHEMA: Schema = BooleanSchema::new(
    "Allow to propagate (inherit) permissions.")
    .default(true)
    .schema();

pub const ACL_PATH_SCHEMA: Schema = StringSchema::new(
    "Access control path.")
    .format(&ACL_PATH_FORMAT)
    .min_length(1)
    .max_length(128)
    .schema();

pub const ACL_UGID_TYPE_SCHEMA: Schema = StringSchema::new(
    "Type of 'ugid' property.")
    .format(&ApiStringFormat::Enum(&["user", "group"]))
    .schema();

pub const ACL_ROLE_SCHEMA: Schema = StringSchema::new(
    "Role.")
    .format(&ApiStringFormat::Enum(&["Admin", "User", "Audit", "NoAccess"]))
    .schema();

#[api(
    properties: {
        propagate: {
            schema: ACL_PROPAGATE_SCHEMA,
        },
 	path: {
            schema: ACL_PATH_SCHEMA,
        },
        ugid_type: {
            schema: ACL_UGID_TYPE_SCHEMA,
        },
	ugid: {
            type: String,
            description: "User or Group ID.",
        },
	roleid: {
            schema: ACL_ROLE_SCHEMA,
        }
    }
)]
#[derive(Serialize, Deserialize)]
/// ACL list entry.
pub struct AclListItem {
    path: String,
    ugid: String,
    ugid_type: String,
    propagate: bool,
    roleid: String,
}

fn extract_acl_node_data(
    node: &acl::AclTreeNode,
    path: &str,
    list: &mut Vec<AclListItem>,
) {
    for (user, roles) in &node.users {
        for (role, propagate) in roles {
            list.push(AclListItem {
                path: if path.is_empty() { String::from("/") } else { path.to_string() },
                propagate: *propagate,
                ugid_type: String::from("user"),
                ugid: user.to_string(),
                roleid: role.to_string(),
            });
        }
    }
    for (group, roles) in &node.groups {
        for (role, propagate) in roles {
            list.push(AclListItem {
                path: if path.is_empty() { String::from("/") } else { path.to_string() },
                propagate: *propagate,
                ugid_type: String::from("group"),
                ugid: group.to_string(),
                roleid: role.to_string(),
            });
        }
    }
    for (comp, child) in &node.children {
        let new_path = format!("{}/{}", path, comp);
        extract_acl_node_data(child, &new_path, list);
    }
}

#[api(
    returns: {
        description: "ACL entry list.",
        type: Array,
        items: {
            type: AclListItem,
        }
    }
)]
/// Read Access Control List (ACLs).
pub fn read_acl(
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<AclListItem>, Error> {

    //let auth_user = rpcenv.get_user().unwrap();

    let (tree, digest) = acl::config()?;

    let mut list: Vec<AclListItem> = Vec::new();
    extract_acl_node_data(&tree.root, "", &mut list);

    Ok(list)
}

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_READ_ACL);
