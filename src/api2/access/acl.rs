use anyhow::{bail, Error};
use ::serde::{Deserialize, Serialize};

use proxmox::api::{api, Router, RpcEnvironment, Permission};
use proxmox::tools::fs::open_file_locked;

use crate::api2::types::*;
use crate::config::acl;
use crate::config::acl::{Role, PRIV_SYS_AUDIT, PRIV_PERMISSIONS_MODIFY};

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
            type: Role,
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
    exact: bool,
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
    if exact {
        return;
    }
    for (comp, child) in &node.children {
        let new_path = format!("{}/{}", path, comp);
        extract_acl_node_data(child, &new_path, list, exact);
    }
}

#[api(
    input: {
        properties: {
	    path: {
                schema: ACL_PATH_SCHEMA,
                optional: true,
            },
            exact: {
                description: "If set, returns only ACL for the exact path.",
                type: bool,
                optional: true,
                default: false,
            },
        },
    },
    returns: {
        description: "ACL entry list.",
        type: Array,
        items: {
            type: AclListItem,
        }
    },
    access: {
        permission: &Permission::Privilege(&["access", "acl"], PRIV_SYS_AUDIT, false),
    },
)]
/// Read Access Control List (ACLs).
pub fn read_acl(
    path: Option<String>,
    exact: bool,
    mut rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<AclListItem>, Error> {

    //let auth_user = rpcenv.get_user().unwrap();

    let (mut tree, digest) = acl::config()?;

    let mut list: Vec<AclListItem> = Vec::new();
    if let Some(path) = &path {
        if let Some(node) = &tree.find_node(path) {
            extract_acl_node_data(&node, path, &mut list, exact);
        }
    } else {
        extract_acl_node_data(&tree.root, "", &mut list, exact);
    }

    rpcenv["digest"] = proxmox::tools::digest_to_hex(&digest).into();

    Ok(list)
}

#[api(
    protected: true,
    input: {
        properties: {
	    path: {
                schema: ACL_PATH_SCHEMA,
            },
	    role: {
                type: Role,
            },
            propagate: {
                optional: true,
                schema: ACL_PROPAGATE_SCHEMA,
            },
            auth_id: {
                optional: true,
                type: Authid,
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
    access: {
        permission: &Permission::Privilege(&["access", "acl"], PRIV_PERMISSIONS_MODIFY, false),
    },
)]
/// Update Access Control List (ACLs).
pub fn update_acl(
    path: String,
    role: String,
    propagate: Option<bool>,
    auth_id: Option<Authid>,
    group: Option<String>,
    delete: Option<bool>,
    digest: Option<String>,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {

    let _lock = open_file_locked(acl::ACL_CFG_LOCKFILE, std::time::Duration::new(10, 0), true)?;

    let (mut tree, expected_digest) = acl::config()?;

    if let Some(ref digest) = digest {
        let digest = proxmox::tools::hex_to_digest(digest)?;
        crate::tools::detect_modified_configuration_file(&digest, &expected_digest)?;
    }

    let propagate = propagate.unwrap_or(true);

    let delete = delete.unwrap_or(false);

    if let Some(ref _group) = group {
        bail!("parameter 'group' - groups are currently not supported.");
    } else if let Some(ref auth_id) = auth_id {
        if !delete { // Note: we allow to delete non-existent users
            let user_cfg = crate::config::user::cached_config()?;
            if user_cfg.sections.get(&auth_id.to_string()).is_none() {
                bail!(format!("no such {}.",
                              if auth_id.is_token() { "API token" } else { "user" }));
            }
        }
    } else {
        bail!("missing 'userid' or 'group' parameter.");
    }

    if !delete { // Note: we allow to delete entries with invalid path
        acl::check_acl_path(&path)?;
    }

    if let Some(auth_id) = auth_id {
        if delete {
            tree.delete_user_role(&path, &auth_id, &role);
        } else {
            tree.insert_user_role(&path, &auth_id, &role, propagate);
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
