use std::sync::Arc;

use anyhow::Error;

use pbs_api_types::{
    Authid, BackupNamespace, PRIV_DATASTORE_AUDIT, PRIV_DATASTORE_BACKUP, PRIV_DATASTORE_MODIFY,
    PRIV_DATASTORE_READ,
};
use pbs_config::CachedUserInfo;
use pbs_datastore::{backup_info::BackupGroup, DataStore, ListGroups, ListNamespacesRecursive};

/// A priviledge aware iterator for all backup groups in all Namespaces below an anchor namespace,
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
            store: store,
            user_info: CachedUserInfo::new()?,
        })
    }
}

static NS_PRIVS_OK: u64 =
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
                            match self.store.owns_backup(
                                &group.backup_ns(),
                                group.group(),
                                &auth_id,
                            ) {
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
                            let privs = if ns.is_root() {
                                info.lookup_privs(&auth_id, &["datastore", self.store.name()])
                            } else {
                                info.lookup_privs(
                                    &auth_id,
                                    &["datastore", self.store.name(), &ns.to_string()],
                                )
                            };
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
                        self.state = match ListGroups::new(Arc::clone(&self.store), ns) {
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
