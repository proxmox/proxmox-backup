use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use anyhow::{bail, format_err, Error};

use pbs_api_types::{BackupNamespace, BackupType, BACKUP_DATE_REGEX, BACKUP_ID_REGEX};

use crate::backup_info::{BackupDir, BackupGroup};
use crate::DataStore;

/// A iterator for all BackupDir's (Snapshots) in a BackupGroup
pub struct ListSnapshots {
    group: BackupGroup,
    fd: proxmox_sys::fs::ReadDir,
}

impl ListSnapshots {
    pub fn new(group: BackupGroup) -> Result<Self, Error> {
        let group_path = group.full_group_path();
        Ok(ListSnapshots {
            fd: proxmox_sys::fs::read_subdir(libc::AT_FDCWD, &group_path)
                .map_err(|err| format_err!("read dir {group_path:?} - {err}"))?,
            group,
        })
    }
}

impl Iterator for ListSnapshots {
    type Item = Result<BackupDir, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let item = self.fd.next()?; // either get a entry to check or return None if exhausted
            let entry = match item {
                Ok(ref entry) => {
                    match entry.file_type() {
                        Some(nix::dir::Type::Directory) => entry, // OK
                        _ => continue,
                    }
                }
                Err(err) => return Some(Err(err)),
            };
            if let Ok(name) = entry.file_name().to_str() {
                if BACKUP_DATE_REGEX.is_match(name) {
                    let backup_time = match proxmox_time::parse_rfc3339(name) {
                        Ok(time) => time,
                        Err(err) => return Some(Err(err)),
                    };

                    return Some(BackupDir::with_group(self.group.clone(), backup_time));
                }
            }
        }
    }
}

/// A iterator for a (single) level of Backup Groups
pub struct ListGroups {
    store: Arc<DataStore>,
    ns: BackupNamespace,
    type_fd: proxmox_sys::fs::ReadDir,
    id_state: Option<(BackupType, proxmox_sys::fs::ReadDir)>,
}

impl ListGroups {
    pub fn new(store: Arc<DataStore>, ns: BackupNamespace) -> Result<Self, Error> {
        let mut base_path = store.base_path();
        base_path.push(ns.path());
        Ok(ListGroups {
            type_fd: proxmox_sys::fs::read_subdir(libc::AT_FDCWD, &base_path)?,
            store,
            ns,
            id_state: None,
        })
    }
}

impl Iterator for ListGroups {
    type Item = Result<BackupGroup, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some((group_type, ref mut id_fd)) = self.id_state {
                let item = match id_fd.next() {
                    Some(item) => item,
                    None => {
                        self.id_state = None;
                        continue; // exhausted all IDs for the current group type, try others
                    }
                };
                let entry = match item {
                    Ok(ref entry) => {
                        match entry.file_type() {
                            Some(nix::dir::Type::Directory) => entry, // OK
                            _ => continue,
                        }
                    }
                    Err(err) => return Some(Err(err)),
                };
                if let Ok(name) = entry.file_name().to_str() {
                    if BACKUP_ID_REGEX.is_match(name) {
                        return Some(Ok(BackupGroup::new(
                            Arc::clone(&self.store),
                            self.ns.clone(),
                            (group_type, name.to_owned()).into(),
                        )));
                    }
                }
            } else {
                let item = self.type_fd.next()?;
                let entry = match item {
                    // filter directories
                    Ok(ref entry) => {
                        match entry.file_type() {
                            Some(nix::dir::Type::Directory) => entry, // OK
                            _ => continue,
                        }
                    }
                    Err(err) => return Some(Err(err)),
                };
                if let Ok(name) = entry.file_name().to_str() {
                    if let Ok(group_type) = BackupType::from_str(name) {
                        // found a backup group type, descend into it to scan all IDs in it
                        // by switching to the id-state branch
                        let base_fd = entry.parent_fd();
                        let id_dirfd = match proxmox_sys::fs::read_subdir(base_fd, name) {
                            Ok(dirfd) => dirfd,
                            Err(err) => return Some(Err(err.into())),
                        };
                        self.id_state = Some((group_type, id_dirfd));
                    }
                }
            }
        }
    }
}

/// A iterator for a (single) level of Namespaces
pub struct ListNamespaces {
    ns: BackupNamespace,
    base_path: PathBuf,
    ns_state: Option<proxmox_sys::fs::ReadDir>,
}

impl ListNamespaces {
    /// construct a new single-level namespace iterator on a datastore with an optional anchor ns
    pub fn new(store: Arc<DataStore>, ns: BackupNamespace) -> Result<Self, Error> {
        Ok(ListNamespaces {
            ns,
            base_path: store.base_path(),
            ns_state: None,
        })
    }

    /// to allow constructing the iter directly on a path, e.g., provided by section config
    ///
    /// NOTE: it's recommended to use the datastore one constructor or go over the recursive iter
    pub fn new_from_path(path: PathBuf, ns: Option<BackupNamespace>) -> Result<Self, Error> {
        Ok(ListNamespaces {
            ns: ns.unwrap_or_default(),
            base_path: path,
            ns_state: None,
        })
    }
}

impl Iterator for ListNamespaces {
    type Item = Result<BackupNamespace, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(ref mut id_fd) = self.ns_state {
                let item = id_fd.next()?; // if this returns none we are done
                let entry = match item {
                    Ok(ref entry) => {
                        match entry.file_type() {
                            Some(nix::dir::Type::Directory) => entry, // OK
                            _ => continue,
                        }
                    }
                    Err(err) => return Some(Err(err)),
                };
                if let Ok(name) = entry.file_name().to_str() {
                    if name != "." && name != ".." {
                        return Some(BackupNamespace::from_parent_ns(&self.ns, name.to_string()));
                    }
                }
                continue; // file did not match regex or isn't valid utf-8
            } else {
                let mut base_path = self.base_path.to_owned();
                if !self.ns.is_root() {
                    base_path.push(self.ns.path());
                }
                base_path.push("ns");

                let ns_dirfd = match proxmox_sys::fs::read_subdir(libc::AT_FDCWD, &base_path) {
                    Ok(dirfd) => dirfd,
                    Err(nix::Error::Sys(nix::errno::Errno::ENOENT)) => return None,
                    Err(err) => return Some(Err(err.into())),
                };
                // found a ns directory, descend into it to scan all it's namespaces
                self.ns_state = Some(ns_dirfd);
            }
        }
    }
}

/// A iterator for all Namespaces below an anchor namespace, most often that will be the
/// `BackupNamespace::root()` one.
///
/// Descends depth-first (pre-order) into the namespace hierachy yielding namespaces immediately as
/// it finds them.
///
/// Note: The anchor namespaces passed on creating the iterator will yielded as first element, this
/// can be usefull for searching all backup groups from a certain anchor, as that can contain
/// sub-namespaces but also groups on its own level, so otherwise one would need to special case
/// the ones from the own level.
pub struct ListNamespacesRecursive {
    store: Arc<DataStore>,
    /// the starting namespace we search downward from
    ns: BackupNamespace,
    /// the maximal recursion depth from the anchor start ns (depth == 0) downwards
    max_depth: u8,
    state: Option<Vec<ListNamespaces>>, // vector to avoid code recursion
}

impl ListNamespacesRecursive {
    /// Creates an recursive namespace iterator.
    pub fn new(store: Arc<DataStore>, ns: BackupNamespace) -> Result<Self, Error> {
        Self::new_max_depth(store, ns, pbs_api_types::MAX_NAMESPACE_DEPTH)
    }

    /// Creates an recursive namespace iterator that iterates recursively until depth is reached.
    ///
    /// `depth` must be smaller than pbs_api_types::MAX_NAMESPACE_DEPTH.
    ///
    /// Depth is counted relatively, that means not from the datastore as anchor, but from `ns`,
    /// and it will be clamped to `min(depth, MAX_NAMESPACE_DEPTH - ns.depth())` automatically.
    pub fn new_max_depth(
        store: Arc<DataStore>,
        ns: BackupNamespace,
        max_depth: usize,
    ) -> Result<Self, Error> {
        if max_depth > pbs_api_types::MAX_NAMESPACE_DEPTH {
            let limit = pbs_api_types::MAX_NAMESPACE_DEPTH + 1;
            bail!("depth must be smaller than {limit}");
        }
        // always clamp, but don't error if we violated relative depth, makes it simpler to use.
        let max_depth = std::cmp::min(max_depth, pbs_api_types::MAX_NAMESPACE_DEPTH - ns.depth());

        Ok(ListNamespacesRecursive {
            store,
            ns,
            max_depth: max_depth as u8,
            state: None,
        })
    }
}

impl Iterator for ListNamespacesRecursive {
    type Item = Result<BackupNamespace, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(ref mut state) = self.state {
                if state.is_empty() {
                    return None; // there's a state but it's empty -> we're all done
                }
                let iter = match state.last_mut() {
                    Some(iter) => iter,
                    None => return None, // unexpected, should we just unwrap?
                };
                match iter.next() {
                    Some(Ok(ns)) => {
                        if state.len() < self.max_depth as usize {
                            match ListNamespaces::new(Arc::clone(&self.store), ns.to_owned()) {
                                Ok(iter) => state.push(iter),
                                Err(err) => log::error!("failed to create child ns iter {err}"),
                            }
                        }
                        return Some(Ok(ns));
                    }
                    Some(ns_err) => return Some(ns_err),
                    None => {
                        let _ = state.pop(); // done at this (and belows) level, continue in parent
                    }
                }
            } else {
                // first next call ever: initialize state vector and start iterating at our level
                let mut state = Vec::with_capacity(pbs_api_types::MAX_NAMESPACE_DEPTH);
                if self.max_depth as usize > 0 {
                    match ListNamespaces::new(Arc::clone(&self.store), self.ns.to_owned()) {
                        Ok(list_ns) => state.push(list_ns),
                        Err(err) => {
                            // yield the error but set the state to Some to avoid re-try, a future
                            // next() will then see the state, and the empty check yield's None
                            self.state = Some(state);
                            return Some(Err(err));
                        }
                    }
                }
                self.state = Some(state);
                return Some(Ok(self.ns.to_owned())); // return our anchor ns for convenience
            }
        }
    }
}
