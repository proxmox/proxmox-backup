use anyhow::{bail, Error};
use serde_json::Value;

use pbs_config::CachedUserInfo;
use proxmox_router::{http_bail, ApiMethod, Permission, Router, RpcEnvironment};
use proxmox_schema::*;

use pbs_api_types::{
    Authid, BackupNamespace, NamespaceListItem, Operation, DATASTORE_SCHEMA, NS_MAX_DEPTH_SCHEMA,
    PRIV_DATASTORE_AUDIT, PRIV_DATASTORE_BACKUP, PRIV_DATASTORE_MODIFY, PROXMOX_SAFE_ID_FORMAT,
};

use pbs_datastore::DataStore;

#[api(
    input: {
        properties: {
            store: {
                schema: DATASTORE_SCHEMA,
            },
            name: {
                type: String,
                description: "The name of the new namespace to add at the parent.",
                format: &PROXMOX_SAFE_ID_FORMAT,
                min_length: 1,
                max_length: 32,
            },
            parent: {
                type: BackupNamespace,
                //description: "To list only namespaces below the passed one.",
                optional: true,
            },
        },
    },
    returns: pbs_api_types::ADMIN_DATASTORE_LIST_NAMESPACE_RETURN_TYPE,
    access: {
        permission: &Permission::Or(&[
            &Permission::Privilege(
                &["datastore", "{store}"],
                PRIV_DATASTORE_MODIFY,
                true,
            ),
            &Permission::Privilege(
                &["datastore", "{store}", "{parent}"],
                PRIV_DATASTORE_MODIFY,
                true,
            ),
        ])
    },
)]
/// List the namespaces of a datastore.
pub fn create_namespace(
    store: String,
    name: String,
    parent: Option<BackupNamespace>,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<BackupNamespace, Error> {
    let parent = parent.unwrap_or_default();

    let datastore = DataStore::lookup_datastore(&store, Some(Operation::Read))?;

    datastore.create_namespace(&parent, name)
}

#[api(
    input: {
        properties: {
            store: {
                schema: DATASTORE_SCHEMA,
            },
            parent: {
                type: BackupNamespace,
                // FIXME: fix the api macro stuff to finally allow that ... -.-
                //description: "To list only namespaces below the passed one.",
                optional: true,
            },
            "max-depth": {
                schema: NS_MAX_DEPTH_SCHEMA,
                optional: true,
            },
        },
    },
    returns: pbs_api_types::ADMIN_DATASTORE_LIST_NAMESPACE_RETURN_TYPE,
    access: {
        permission: &Permission::Or(&[
            &Permission::Privilege(
                &["datastore", "{store}"],
                PRIV_DATASTORE_BACKUP | PRIV_DATASTORE_AUDIT,
                true,
            ),
            &Permission::Privilege(
                &["datastore", "{store}", "{parent}"],
                PRIV_DATASTORE_BACKUP | PRIV_DATASTORE_AUDIT,
                true,
            ),
        ])
    },
)]
/// List the namespaces of a datastore.
pub fn list_namespaces(
    store: String,
    parent: Option<BackupNamespace>,
    max_depth: Option<usize>,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<NamespaceListItem>, Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let user_info = CachedUserInfo::new()?;

    let datastore = DataStore::lookup_datastore(&store, Some(Operation::Read))?;
    let parent = parent.unwrap_or_default();

    let ns_to_item =
        |ns: BackupNamespace| -> NamespaceListItem { NamespaceListItem { ns, comment: None } };

    Ok(datastore
        .recursive_iter_backup_ns_ok(parent, max_depth)?
        .filter(|ns| {
            if ns.is_root() {
                return true; // already covered by access permission above
            }
            let privs = user_info.lookup_privs(&auth_id, &["datastore", &store, &ns.to_string()]);
            privs & (PRIV_DATASTORE_BACKUP | PRIV_DATASTORE_AUDIT) != 0
        })
        .map(ns_to_item)
        .collect())
}

#[api(
    input: {
        properties: {
            store: { schema: DATASTORE_SCHEMA },
            ns: {
                type: BackupNamespace,
            },
        },
    },
    access: {
        permission: &Permission::Anybody,
    },
)]
/// Delete a backup namespace including all snapshots.
pub fn delete_namespace(
    store: String,
    ns: BackupNamespace,
    _info: &ApiMethod,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let user_info = CachedUserInfo::new()?;

    // we could allow it as easy purge-whole datastore, but lets be more restrictive for now
    if ns.is_root() {
        bail!("cannot delete root namespace!");
    };

    let parent = ns.parent(); // must have MODIFY permission on parent to allow deletion
    let user_privs = if parent.is_root() {
        user_info.lookup_privs(&auth_id, &["datastore", &store])
    } else {
        user_info.lookup_privs(&auth_id, &["datastore", &store, &parent.to_string()])
    };
    if (user_privs & PRIV_DATASTORE_MODIFY) == 0 {
        http_bail!(FORBIDDEN, "permission check failed");
    }

    let datastore = DataStore::lookup_datastore(&store, Some(Operation::Write))?;

    if !datastore.remove_namespace_recursive(&ns)? {
        bail!("group only partially deleted due to protected snapshots");
    }

    Ok(Value::Null)
}

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_LIST_NAMESPACES)
    .post(&API_METHOD_CREATE_NAMESPACE)
    .delete(&API_METHOD_DELETE_NAMESPACE);
