use std::sync::Arc;

use anyhow::Error;

use pbs_api_types::{
    Authid, BackupNamespace, PRIV_DATASTORE_AUDIT, PRIV_DATASTORE_BACKUP, PRIV_DATASTORE_MODIFY,
};
use pbs_config::CachedUserInfo;
use pbs_datastore::{backup_info::BackupGroup, DataStore, ListGroups, ListNamespacesRecursive};

/// A priviledge aware iterator for all backup groups in all Namespaces below an anchor namespace,
/// most often that will be the `BackupNamespace::root()` one.
///
/// Is basically just a filter-iter for pbs_datastore::ListNamespacesRecursive including access and
/// optional owner checks.
pub struct ListAccessibleBackupGroups<'a> {
    store: Arc<DataStore>,
    auth_id: Option<&'a Authid>,
    user_info: Arc<CachedUserInfo>,
    state: Option<ListGroups>,
    ns_iter: ListNamespacesRecursive,
}

impl <'a> ListAccessibleBackupGroups<'a> {
    // TODO: builder pattern

    pub fn new(
        store: Arc<DataStore>,
        ns: BackupNamespace,
        max_depth: usize,
        auth_id: Option<&'a Authid>,
    ) -> Result<Self, Error> {
        let ns_iter = ListNamespacesRecursive::new_max_depth(Arc::clone(&store), ns, max_depth)?;
        Ok(ListAccessibleBackupGroups {
            auth_id,
            ns_iter,
            state: None,
            store: store,
            user_info: CachedUserInfo::new()?,
        })
    }
}

impl <'a> Iterator for ListAccessibleBackupGroups<'a> {
    type Item = Result<BackupGroup, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        const PRIVS_OK: u64 = PRIV_DATASTORE_MODIFY | PRIV_DATASTORE_BACKUP | PRIV_DATASTORE_AUDIT;
        loop {
            if let Some(ref mut state) = self.state {
                match state.next() {
                    Some(Ok(group)) => {
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
                            if privs & PRIVS_OK == 0 {
                                continue;
                            }
                        }
                        self.state = match ListGroups::new(Arc::clone(&self.store), ns) {
                            Ok(iter) => Some(iter),
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
