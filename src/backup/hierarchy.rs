use std::sync::Arc;

use anyhow::{bail, Error};

use pbs_api_types::{
    privs_to_priv_names, Authid, BackupNamespace, PRIV_DATASTORE_AUDIT, PRIV_DATASTORE_BACKUP,
    PRIV_DATASTORE_MODIFY, PRIV_DATASTORE_READ,
};
use pbs_config::CachedUserInfo;
use pbs_datastore::{backup_info::BackupGroup, DataStore, ListGroups, ListNamespacesRecursive};

/// Asserts that `privs` are fulfilled on datastore + (optional) namespace.
pub fn check_ns_privs(
    store: &str,
    ns: &BackupNamespace,
    auth_id: &Authid,
    privs: u64,
) -> Result<(), Error> {
    check_ns_privs_full(store, ns, auth_id, privs, 0).map(|_| ())
}

/// Asserts that `privs` for creating/destroying namespace in datastore are fulfilled.
pub fn check_ns_modification_privs(
    store: &str,
    ns: &BackupNamespace,
    auth_id: &Authid,
) -> Result<(), Error> {
    // we could allow it as easy purge-whole datastore, but lets be more restrictive for now
    if ns.is_root() {
        // TODO
        bail!("Cannot create/delete root namespace!");
    }

    let parent = ns.parent();

    check_ns_privs(store, &parent, auth_id, PRIV_DATASTORE_MODIFY)
}

/// Asserts that either either `full_access_privs` or `partial_access_privs` are fulfilled on
/// datastore + (optional) namespace.
///
/// Return value indicates whether further checks like group ownerships are required because
/// `full_access_privs` are missing.
pub fn check_ns_privs_full(
    store: &str,
    ns: &BackupNamespace,
    auth_id: &Authid,
    full_access_privs: u64,
    partial_access_privs: u64,
) -> Result<bool, Error> {
    let user_info = CachedUserInfo::new()?;
    let acl_path = ns.acl_path(store);
    let privs = user_info.lookup_privs(auth_id, &acl_path);

    if full_access_privs != 0 && (privs & full_access_privs) != 0 {
        return Ok(false);
    }
    if partial_access_privs != 0 && (privs & partial_access_privs) != 0 {
        return Ok(true);
    }

    let priv_names = privs_to_priv_names(full_access_privs | partial_access_privs).join("|");
    let path = format!("/{}", acl_path.join("/"));

    proxmox_router::http_bail!(
        FORBIDDEN,
        "permission check failed - missing {priv_names} on {path}"
    );
}

pub fn can_access_any_namespace(
    store: Arc<DataStore>,
    auth_id: &Authid,
    user_info: &CachedUserInfo,
) -> bool {
    // NOTE: traversing the datastore could be avoided if we had an "ACL tree: is there any priv
    // below /datastore/{store}" helper
    let mut iter =
        if let Ok(iter) = store.recursive_iter_backup_ns_ok(BackupNamespace::root(), None) {
            iter
        } else {
            return false;
        };
    let wanted =
        PRIV_DATASTORE_AUDIT | PRIV_DATASTORE_MODIFY | PRIV_DATASTORE_READ | PRIV_DATASTORE_BACKUP;
    let name = store.name();
    iter.any(|ns| -> bool {
        let user_privs = user_info.lookup_privs(auth_id, &["datastore", name, &ns.to_string()]);
        user_privs & wanted != 0
    })
}

/// A privilege aware iterator for all backup groups in all Namespaces below an anchor namespace,
/// most often that will be the `BackupNamespace::root()` one.
///
/// Is basically just a filter-iter for pbs_datastore::ListNamespacesRecursive including access and
/// optional owner checks.
pub struct ListAccessibleBackupGroups<'a> {
    store: &'a Arc<DataStore>,
    auth_id: Option<&'a Authid>,
    user_info: Arc<CachedUserInfo>,
    /// The priv on NS level that allows auth_id trump the owner check
    override_owner_priv: u64,
    /// The priv that auth_id is required to have on NS level additionally to being owner
    owner_and_priv: u64,
    /// Contains the intertnal state, group iter and a bool flag for override_owner_priv
    state: Option<(ListGroups, bool)>,
    ns_iter: ListNamespacesRecursive,
}

impl<'a> ListAccessibleBackupGroups<'a> {
    // TODO: builder pattern

    pub fn new_owned(
        store: &'a Arc<DataStore>,
        ns: BackupNamespace,
        max_depth: usize,
        auth_id: Option<&'a Authid>,
    ) -> Result<Self, Error> {
        // only owned groups by default and no extra priv required
        Self::new_with_privs(store, ns, max_depth, None, None, auth_id)
    }

    pub fn new_with_privs(
        store: &'a Arc<DataStore>,
        ns: BackupNamespace,
        max_depth: usize,
        override_owner_priv: Option<u64>,
        owner_and_priv: Option<u64>,
        auth_id: Option<&'a Authid>,
    ) -> Result<Self, Error> {
        let ns_iter = ListNamespacesRecursive::new_max_depth(Arc::clone(store), ns, max_depth)?;
        Ok(ListAccessibleBackupGroups {
            auth_id,
            ns_iter,
            override_owner_priv: override_owner_priv.unwrap_or(0),
            owner_and_priv: owner_and_priv.unwrap_or(0),
            state: None,
            store,
            user_info: CachedUserInfo::new()?,
        })
    }
}

pub static NS_PRIVS_OK: u64 =
    PRIV_DATASTORE_MODIFY | PRIV_DATASTORE_READ | PRIV_DATASTORE_BACKUP | PRIV_DATASTORE_AUDIT;

impl<'a> Iterator for ListAccessibleBackupGroups<'a> {
    type Item = Result<BackupGroup, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some((ref mut state, override_owner)) = self.state {
                match state.next() {
                    Some(Ok(group)) => {
                        if override_owner {
                            return Some(Ok(group));
                        }
                        if let Some(auth_id) = &self.auth_id {
                            match self
                                .store
                                .owns_backup(group.backup_ns(), group.group(), auth_id)
                            {
                                Ok(is_owner) if is_owner => return Some(Ok(group)),
                                Ok(_) => continue,
                                Err(err) => return Some(Err(err)),
                            }
                        } else {
                            return Some(Ok(group));
                        }
                    }
                    Some(Err(err)) => return Some(Err(err)),
                    None => {
                        self.state = None; // level exhausted, need to check next NS
                    }
                }
            } else {
                match self.ns_iter.next() {
                    Some(Ok(ns)) => {
                        let mut override_owner = false;
                        if let Some(auth_id) = &self.auth_id {
                            let info = &self.user_info;

                            let privs = info.lookup_privs(auth_id, &ns.acl_path(self.store.name()));

                            if privs & NS_PRIVS_OK == 0 {
                                continue;
                            }

                            // check first if *any* override owner priv is available up front
                            if privs & self.override_owner_priv != 0 {
                                override_owner = true;
                            } else if privs & self.owner_and_priv != self.owner_and_priv {
                                continue; // no owner override and no extra privs -> nothing visible
                            }
                        }
                        self.state = match ListGroups::new(Arc::clone(self.store), ns) {
                            Ok(iter) => Some((iter, override_owner)),
                            Err(err) => return Some(Err(err)),
                        };
                    }
                    Some(Err(err)) => return Some(Err(err)),
                    None => return None, // exhausted with all NS -> done
                }
            }
        }
    }
}
