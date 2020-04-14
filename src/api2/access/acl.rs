use failure::*;
use ::serde::{Deserialize, Serialize};

use proxmox::api::{api, Router, RpcEnvironment};
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
    .format(&ApiStringFormat::Enum(&[
        "Admin",
        "Audit",
        "Datastore.Admin",
        "Datastore.Audit",
        "Datastore.User",
        "NoAccess",
    ]))
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

fn check_acl_path(path: &str) -> Result<(), Error> {

    let path = acl::split_acl_path(path);

    if path.is_empty() { return Ok(()); }

    if path.len() == 2 {
        if path[0] == "storage" { return Ok(()); }
    }

    bail!("invalid acl path.");
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

    // fixme: return digest?
    let (tree, _digest) = acl::config()?;

    let mut list: Vec<AclListItem> = Vec::new();
    extract_acl_node_data(&tree.root, "", &mut list);

    Ok(list)
}

#[api(
    input: {
        properties: {
	    path: {
                schema: ACL_PATH_SCHEMA,
            },
	    role: {
                schema: ACL_ROLE_SCHEMA,
            },
            propagate: {
                optional: true,
                schema: ACL_PROPAGATE_SCHEMA,
            },
            userid: {
                optional: true,
                schema: PROXMOX_USER_ID_SCHEMA,
            },
            group: {
                optional: true,
                schema: PROXMOX_GROUP_ID_SCHEMA,
            },
            delete: {
                optional: true,
                description: "Remove permissions (instead of adding it).",
                type: bool,
            },
            digest: {
                optional: true,
                schema: PROXMOX_CONFIG_DIGEST_SCHEMA,
            },
       },
    },
)]
/// Update Access Control List (ACLs).
pub fn update_acl(
    path: String,
    role: String,
    propagate: Option<bool>,
    userid: Option<String>,
    group: Option<String>,
    delete: Option<bool>,
    digest: Option<String>,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {

    let _lock = crate::tools::open_file_locked(acl::ACL_CFG_LOCKFILE, std::time::Duration::new(10, 0))?;

    let (mut tree, expected_digest) = acl::config()?;

    if let Some(ref digest) = digest {
        let digest = proxmox::tools::hex_to_digest(digest)?;
        crate::tools::detect_modified_configuration_file(&digest, &expected_digest)?;
    }

    let propagate = propagate.unwrap_or(true);

    let delete = delete.unwrap_or(false);

    if let Some(ref _group) = group {
        bail!("parameter 'group' - groups are currently not supported.");
    } else if let Some(ref userid) = userid {
        if !delete { // Note: we allow to delete non-existent users
            let (user_cfg, _) = crate::config::user::cached_config()?;
            if user_cfg.sections.get(userid).is_none() {
                bail!("no such user.");
            }
        }
    } else {
        bail!("missing 'userid' or 'group' parameter.");
    }

    if !delete { // Note: we allow to delete entries with invalid path
        check_acl_path(&path);
    }

    if let Some(userid) = userid {
        if delete {
            tree.delete_user_role(&path, &userid, &role);
        } else {
            tree.insert_user_role(&path, &userid, &role, propagate);
        }
    } else if let Some(group) = group {
        if delete {
            tree.delete_group_role(&path, &group, &role);
        } else {
            tree.insert_group_role(&path, &group, &role, propagate);
        }
    }

    acl::save_config(&tree)?;

    Ok(())
}

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_READ_ACL)
    .put(&API_METHOD_UPDATE_ACL);
