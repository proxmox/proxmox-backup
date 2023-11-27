use anyhow::{bail, Error};
use serde_json::Value;

use pbs_config::CachedUserInfo;
use proxmox_router::{http_bail, ApiMethod, Permission, Router, RpcEnvironment};
use proxmox_schema::*;

use pbs_api_types::{
    Authid, BackupNamespace, NamespaceListItem, Operation, DATASTORE_SCHEMA, NS_MAX_DEPTH_SCHEMA,
    PROXMOX_SAFE_ID_FORMAT,
};

use pbs_datastore::DataStore;

use crate::backup::{check_ns_modification_privs, check_ns_privs, NS_PRIVS_OK};

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
    returns: { type: BackupNamespace },
    access: {
        permission: &Permission::Anybody,
        description: "Requires on /datastore/{store}[/{parent}] DATASTORE_MODIFY"
    },
)]
/// Create a new datastore namespace.
pub fn create_namespace(
    store: String,
    name: String,
    parent: Option<BackupNamespace>,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<BackupNamespace, Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let parent = parent.unwrap_or_default();

    let mut ns = parent.clone();
    ns.push(name.clone())?;

    check_ns_modification_privs(&store, &ns, &auth_id)?;

    let datastore = DataStore::lookup_datastore(&store, Some(Operation::Write))?;

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
        permission: &Permission::Anybody,
        description: "Requires DATASTORE_AUDIT, DATASTORE_MODIFY or DATASTORE_BACKUP /datastore/\
            {store}[/{parent}]",
    },
)]
/// List the namespaces of a datastore.
pub fn list_namespaces(
    store: String,
    parent: Option<BackupNamespace>,
    max_depth: Option<usize>,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<NamespaceListItem>, Error> {
    let parent = parent.unwrap_or_default();
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let user_info = CachedUserInfo::new()?;
    // get result up-front to avoid cloning NS, it's relatively cheap anyway (no IO normally)
    let parent_access = check_ns_privs(&store, &parent, &auth_id, NS_PRIVS_OK);

    let datastore = DataStore::lookup_datastore(&store, Some(Operation::Read))?;

    let iter = match datastore.recursive_iter_backup_ns_ok(parent, max_depth) {
        Ok(iter) => iter,
        // parent NS doesn't exists and user has no privs on it, avoid info leakage.
        Err(_) if parent_access.is_err() => http_bail!(FORBIDDEN, "permission check failed"),
        Err(err) => return Err(err),
    };

    let ns_to_item =
        |ns: BackupNamespace| -> NamespaceListItem { NamespaceListItem { ns, comment: None } };

    let namespace_list: Vec<NamespaceListItem> = iter
        .filter(|ns| {
            let privs = user_info.lookup_privs(&auth_id, &ns.acl_path(&store));
            privs & NS_PRIVS_OK != 0
        })
        .map(ns_to_item)
        .collect();

    if namespace_list.is_empty() && parent_access.is_err() {
        http_bail!(FORBIDDEN, "permission check failed"); // avoid leakage
    }
    Ok(namespace_list)
}

#[api(
    input: {
        properties: {
            store: { schema: DATASTORE_SCHEMA },
            ns: {
                type: BackupNamespace,
            },
            "delete-groups": {
                type: bool,
                description: "If set, all groups will be destroyed in the whole hierarchy below and\
                    including `ns`. If not set, only empty namespaces will be pruned.",
                optional: true,
                default: false,
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
    delete_groups: bool,
    _info: &ApiMethod,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;

    check_ns_modification_privs(&store, &ns, &auth_id)?;

    let datastore = DataStore::lookup_datastore(&store, Some(Operation::Write))?;

    if !datastore.remove_namespace_recursive(&ns, delete_groups)? {
        if delete_groups {
            bail!("group only partially deleted due to protected snapshots");
        } else {
            bail!("only partially deleted due to existing groups but `delete-groups` not true ");
        }
    }

    Ok(Value::Null)
}

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_LIST_NAMESPACES)
    .post(&API_METHOD_CREATE_NAMESPACE)
    .delete(&API_METHOD_DELETE_NAMESPACE);
