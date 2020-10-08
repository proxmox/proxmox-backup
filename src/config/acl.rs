use std::io::Write;
use std::collections::{HashMap, BTreeMap, BTreeSet};
use std::path::{PathBuf, Path};
use std::sync::{Arc, RwLock};
use std::str::FromStr;

use anyhow::{bail, Error};

use lazy_static::lazy_static;

use ::serde::{Deserialize, Serialize};
use serde::de::{value, IntoDeserializer};

use proxmox::tools::{fs::replace_file, fs::CreateOptions};
use proxmox::constnamedbitmap;
use proxmox::api::{api, schema::*};

use crate::api2::types::{Authid,Userid};

// define Privilege bitfield

constnamedbitmap! {
    /// Contains a list of Privileges
    PRIVILEGES: u64 => {
        PRIV_SYS_AUDIT("Sys.Audit");
        PRIV_SYS_MODIFY("Sys.Modify");
        PRIV_SYS_POWER_MANAGEMENT("Sys.PowerManagement");

        PRIV_DATASTORE_AUDIT("Datastore.Audit");
        PRIV_DATASTORE_ALLOCATE("Datastore.Allocate");
        PRIV_DATASTORE_MODIFY("Datastore.Modify");
        PRIV_DATASTORE_READ("Datastore.Read");

        /// Datastore.Backup also requires backup ownership
        PRIV_DATASTORE_BACKUP("Datastore.Backup");
        /// Datastore.Prune also requires backup ownership
        PRIV_DATASTORE_PRUNE("Datastore.Prune");

        PRIV_PERMISSIONS_MODIFY("Permissions.Modify");

        PRIV_REMOTE_AUDIT("Remote.Audit");
        PRIV_REMOTE_MODIFY("Remote.Modify");
        PRIV_REMOTE_READ("Remote.Read");
        PRIV_REMOTE_PRUNE("Remote.Prune");

        PRIV_SYS_CONSOLE("Sys.Console");
    }
}


/// Admin always has all privileges. It can do everything except a few actions
/// which are limited to the 'root@pam` superuser
pub const ROLE_ADMIN: u64 = std::u64::MAX;

/// NoAccess can be used to remove privileges from specific paths
pub const ROLE_NO_ACCESS: u64 = 0;

pub const ROLE_AUDIT: u64 =
PRIV_SYS_AUDIT |
PRIV_DATASTORE_AUDIT;

/// Datastore.Admin can do anything on the datastore.
pub const ROLE_DATASTORE_ADMIN: u64 =
PRIV_DATASTORE_AUDIT |
PRIV_DATASTORE_MODIFY |
PRIV_DATASTORE_READ |
PRIV_DATASTORE_BACKUP |
PRIV_DATASTORE_PRUNE;

/// Datastore.Reader can read datastore content an do restore
pub const ROLE_DATASTORE_READER: u64 =
PRIV_DATASTORE_AUDIT |
PRIV_DATASTORE_READ;

/// Datastore.Backup can do backup and restore, but no prune.
pub const ROLE_DATASTORE_BACKUP: u64 =
PRIV_DATASTORE_BACKUP;

/// Datastore.PowerUser can do backup, restore, and prune.
pub const ROLE_DATASTORE_POWERUSER: u64 =
PRIV_DATASTORE_PRUNE |
PRIV_DATASTORE_BACKUP;

/// Datastore.Audit can audit the datastore.
pub const ROLE_DATASTORE_AUDIT: u64 =
PRIV_DATASTORE_AUDIT;

/// Remote.Audit can audit the remote
pub const ROLE_REMOTE_AUDIT: u64 =
PRIV_REMOTE_AUDIT;

/// Remote.Admin can do anything on the remote.
pub const ROLE_REMOTE_ADMIN: u64 =
PRIV_REMOTE_AUDIT |
PRIV_REMOTE_MODIFY |
PRIV_REMOTE_READ |
PRIV_REMOTE_PRUNE;

/// Remote.SyncOperator can do read and prune on the remote.
pub const ROLE_REMOTE_SYNC_OPERATOR: u64 =
PRIV_REMOTE_AUDIT |
PRIV_REMOTE_READ |
PRIV_REMOTE_PRUNE;

pub const ROLE_NAME_NO_ACCESS: &str ="NoAccess";

#[api()]
#[repr(u64)]
#[derive(Serialize, Deserialize)]
/// Role
pub enum Role {
    /// Administrator
    Admin = ROLE_ADMIN,
    /// Auditor
    Audit = ROLE_AUDIT,
    /// Disable Access
    NoAccess = ROLE_NO_ACCESS,
    /// Datastore Administrator
    DatastoreAdmin = ROLE_DATASTORE_ADMIN,
    /// Datastore Reader (inspect datastore content and do restores)
    DatastoreReader = ROLE_DATASTORE_READER,
    /// Datastore Backup (backup and restore owned backups)
    DatastoreBackup = ROLE_DATASTORE_BACKUP,
    /// Datastore PowerUser (backup, restore and prune owned backup)
    DatastorePowerUser = ROLE_DATASTORE_POWERUSER,
    /// Datastore Auditor
    DatastoreAudit = ROLE_DATASTORE_AUDIT,
    /// Remote Auditor
    RemoteAudit = ROLE_REMOTE_AUDIT,
    /// Remote Administrator
    RemoteAdmin = ROLE_REMOTE_ADMIN,
    /// Syncronisation Opertator
    RemoteSyncOperator = ROLE_REMOTE_SYNC_OPERATOR,
}

impl FromStr for Role {
    type Err = value::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::deserialize(s.into_deserializer())
    }
}

lazy_static! {
    pub static ref ROLE_NAMES: HashMap<&'static str, (u64, &'static str)> = {
        let mut map = HashMap::new();

        let list = match Role::API_SCHEMA {
            Schema::String(StringSchema { format: Some(ApiStringFormat::Enum(list)), .. }) => list,
            _ => unreachable!(),
        };

        for entry in list.iter() {
            let privs: u64 = Role::from_str(entry.value).unwrap() as u64;
            map.insert(entry.value, (privs, entry.description));
        }

        map
    };
}

pub fn split_acl_path(path: &str) -> Vec<&str> {

    let items = path.split('/');

    let mut components = vec![];

    for name in items {
        if name.is_empty() { continue; }
        components.push(name);
    }

    components
}

pub fn check_acl_path(path: &str) -> Result<(), Error> {

    let components = split_acl_path(path);

    let components_len = components.len();

    if components_len == 0 { return Ok(()); }
    match components[0] {
        "access" => {
            if components_len == 1 { return Ok(()); }
            match components[1] {
                "acl" | "users" => {
                    if components_len == 2 { return Ok(()); }
                }
                _ => {},
            }
        }
        "datastore" => {  // /datastore/{store}
            if components_len <= 2 { return Ok(()); }
        }
        "remote" => { // /remote/{remote}/{store}
            if components_len <= 3 { return Ok(()); }
        }
        "system" => {
            if components_len == 1 { return Ok(()); }
            match components[1] {
                "disks" | "log" | "status" | "tasks" | "time" => {
                    if components_len == 2 { return Ok(()); }
                }
                "services" => { // /system/services/{service}
                    if components_len <= 3 { return Ok(()); }
                }
                "network" => {
                    if components_len == 2 { return Ok(()); }
                    match components[2] {
                        "dns" => {
                            if components_len == 3 { return Ok(()); }
                        }
                        "interfaces" => { // /system/network/interfaces/{iface}
                            if components_len <= 4 { return Ok(()); }
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        _ => {}
    }

    bail!("invalid acl path '{}'.", path);
}

pub struct AclTree {
    pub root: AclTreeNode,
}

pub struct AclTreeNode {
    pub users: HashMap<Authid, HashMap<String, bool>>,
    pub groups: HashMap<String, HashMap<String, bool>>,
    pub children: BTreeMap<String, AclTreeNode>,
}

impl AclTreeNode {

    pub fn new() -> Self {
        Self {
            users: HashMap::new(),
            groups: HashMap::new(),
            children: BTreeMap::new(),
        }
    }

    pub fn extract_roles(&self, auth_id: &Authid, all: bool) -> HashMap<String, bool> {
        let user_roles = self.extract_user_roles(auth_id, all);
        if !user_roles.is_empty() || auth_id.is_token() {
            // user privs always override group privs
            return user_roles
        };

        self.extract_group_roles(auth_id.user(), all)
    }

    pub fn extract_user_roles(&self, auth_id: &Authid, all: bool) -> HashMap<String, bool> {

        let mut map = HashMap::new();

        let roles = match self.users.get(auth_id) {
            Some(m) => m,
            None => return map,
        };

        for (role, propagate) in roles {
            if *propagate || all {
                if role == ROLE_NAME_NO_ACCESS {
                    // return a map with a single role 'NoAccess'
                    let mut map = HashMap::new();
                    map.insert(role.to_string(), false);
                    return map;
                }
                map.insert(role.to_string(), *propagate);
            }
        }

        map
    }

    pub fn extract_group_roles(&self, _user: &Userid, all: bool) -> HashMap<String, bool> {

        let mut map = HashMap::new();

        for (_group, roles) in &self.groups {
            let is_member = false; // fixme: check if user is member of the group
            if !is_member { continue; }

            for (role, propagate) in roles {
                if *propagate || all {
                    if role == ROLE_NAME_NO_ACCESS {
                        // return a map with a single role 'NoAccess'
                        let mut map = HashMap::new();
                        map.insert(role.to_string(), false);
                        return map;
                    }
                    map.insert(role.to_string(), *propagate);
                }
            }
        }

        map
    }

    pub fn delete_group_role(&mut self, group: &str, role: &str) {
        let roles = match self.groups.get_mut(group) {
            Some(r) => r,
            None => return,
        };
        roles.remove(role);
    }

    pub fn delete_user_role(&mut self, auth_id: &Authid, role: &str) {
        let roles = match self.users.get_mut(auth_id) {
            Some(r) => r,
            None => return,
        };
        roles.remove(role);
    }

    pub fn insert_group_role(&mut self, group: String, role: String, propagate: bool) {
        let map = self.groups.entry(group).or_insert_with(|| HashMap::new());
        if role == ROLE_NAME_NO_ACCESS {
            map.clear();
            map.insert(role, propagate);
        } else {
            map.remove(ROLE_NAME_NO_ACCESS);
            map.insert(role, propagate);
        }
    }

    pub fn insert_user_role(&mut self, auth_id: Authid, role: String, propagate: bool) {
        let map = self.users.entry(auth_id).or_insert_with(|| HashMap::new());
        if role == ROLE_NAME_NO_ACCESS {
            map.clear();
            map.insert(role, propagate);
        } else {
            map.remove(ROLE_NAME_NO_ACCESS);
            map.insert(role, propagate);
        }
    }
}

impl AclTree {

    pub fn new() -> Self {
        Self {
            root: AclTreeNode::new(),
        }
    }

    pub fn find_node(&mut self, path: &str) -> Option<&mut AclTreeNode> {
        let path = split_acl_path(path);
        return self.get_node(&path);
    }

    fn get_node(&mut self, path: &[&str]) -> Option<&mut AclTreeNode> {
        let mut node = &mut self.root;
        for comp in path {
            node = match node.children.get_mut(*comp) {
                Some(n) => n,
                None => return None,
            };
        }
        Some(node)
    }

    fn get_or_insert_node(&mut self, path: &[&str]) -> &mut AclTreeNode {
        let mut node = &mut self.root;
        for comp in path {
            node = node.children.entry(String::from(*comp))
                .or_insert_with(|| AclTreeNode::new());
        }
        node
    }

    pub fn delete_group_role(&mut self, path: &str, group: &str, role: &str) {
        let path = split_acl_path(path);
        let node = match self.get_node(&path) {
            Some(n) => n,
            None => return,
        };
        node.delete_group_role(group, role);
    }

    pub fn delete_user_role(&mut self, path: &str, auth_id: &Authid, role: &str) {
        let path = split_acl_path(path);
        let node = match self.get_node(&path) {
            Some(n) => n,
            None => return,
        };
        node.delete_user_role(auth_id, role);
    }

    pub fn insert_group_role(&mut self, path: &str, group: &str, role: &str, propagate: bool) {
        let path = split_acl_path(path);
        let node = self.get_or_insert_node(&path);
        node.insert_group_role(group.to_string(), role.to_string(), propagate);
    }

    pub fn insert_user_role(&mut self, path: &str, auth_id: &Authid, role: &str, propagate: bool) {
        let path = split_acl_path(path);
        let node = self.get_or_insert_node(&path);
        node.insert_user_role(auth_id.to_owned(), role.to_string(), propagate);
    }

    fn write_node_config(
        node: &AclTreeNode,
        path: &str,
        w: &mut dyn Write,
    ) -> Result<(), Error> {

        let mut role_ug_map0 = HashMap::new();
        let mut role_ug_map1 = HashMap::new();

        for (auth_id, roles) in &node.users {
            // no need to save, because root is always 'Administrator'
            if !auth_id.is_token() && auth_id.user() == "root@pam" { continue; }
            for (role, propagate) in roles {
                let role = role.as_str();
                let auth_id = auth_id.to_string();
                if *propagate {
                    role_ug_map1.entry(role).or_insert_with(|| BTreeSet::new())
                        .insert(auth_id);
                } else {
                    role_ug_map0.entry(role).or_insert_with(|| BTreeSet::new())
                        .insert(auth_id);
                }
            }
        }

        for (group, roles) in &node.groups {
            for (role, propagate) in roles {
                let group = format!("@{}", group);
                if *propagate {
                    role_ug_map1.entry(role).or_insert_with(|| BTreeSet::new())
                        .insert(group);
                } else {
                    role_ug_map0.entry(role).or_insert_with(|| BTreeSet::new())
                        .insert(group);
                }
            }
        }

        fn group_by_property_list(
            item_property_map: &HashMap<&str, BTreeSet<String>>,
        ) -> BTreeMap<String, BTreeSet<String>> {
            let mut result_map = BTreeMap::new();
            for (item, property_map) in item_property_map {
                let item_list = property_map.iter().fold(String::new(), |mut acc, v| {
                    if !acc.is_empty() { acc.push(','); }
                    acc.push_str(v);
                    acc
                });
                result_map.entry(item_list).or_insert_with(|| BTreeSet::new())
                    .insert(item.to_string());
            }
            result_map
        }

        let uglist_role_map0 = group_by_property_list(&role_ug_map0);
        let uglist_role_map1 = group_by_property_list(&role_ug_map1);

        fn role_list(roles: &BTreeSet<String>) -> String {
            if roles.contains(ROLE_NAME_NO_ACCESS) { return String::from(ROLE_NAME_NO_ACCESS); }
            roles.iter().fold(String::new(), |mut acc, v| {
                if !acc.is_empty() { acc.push(','); }
                acc.push_str(v);
                acc
            })
        }

        for (uglist, roles) in &uglist_role_map0 {
            let role_list = role_list(roles);
            writeln!(w, "acl:0:{}:{}:{}", if path.is_empty() { "/" } else { path }, uglist, role_list)?;
        }

        for (uglist, roles) in &uglist_role_map1 {
            let role_list = role_list(roles);
            writeln!(w, "acl:1:{}:{}:{}", if path.is_empty() { "/" } else { path }, uglist, role_list)?;
        }

        for (name, child) in node.children.iter() {
            let child_path = format!("{}/{}", path, name);
            Self::write_node_config(child, &child_path, w)?;
        }

        Ok(())
    }

    pub fn write_config(&self, w: &mut dyn Write) -> Result<(), Error> {
        Self::write_node_config(&self.root, "", w)
    }

    fn parse_acl_line(&mut self, line: &str) -> Result<(), Error> {

        let items: Vec<&str> = line.split(':').collect();

        if items.len() != 5 {
            bail!("wrong number of items.");
        }

        if items[0] != "acl" {
            bail!("line does not start with 'acl'.");
        }

        let propagate = if items[1] == "0" {
            false
        } else if items[1] == "1" {
            true
        } else {
            bail!("expected '0' or '1' for propagate flag.");
        };

        let path_str = items[2];
        let path = split_acl_path(path_str);
        let node = self.get_or_insert_node(&path);

        let uglist: Vec<&str> = items[3].split(',').map(|v| v.trim()).collect();

        let rolelist: Vec<&str> = items[4].split(',').map(|v| v.trim()).collect();

        for user_or_group in &uglist {
            for role in &rolelist {
                if !ROLE_NAMES.contains_key(role) {
                    bail!("unknown role '{}'", role);
                }
                if user_or_group.starts_with('@') {
                    let group = &user_or_group[1..];
                    node.insert_group_role(group.to_string(), role.to_string(), propagate);
                } else {
                    node.insert_user_role(user_or_group.parse()?, role.to_string(), propagate);
                }
            }
        }

        Ok(())
    }

    pub fn load(filename: &Path) -> Result<(Self, [u8;32]), Error> {
        let mut tree = Self::new();

        let raw = match std::fs::read_to_string(filename) {
            Ok(v) => v,
            Err(err) => {
                if err.kind() == std::io::ErrorKind::NotFound {
                    String::new()
                } else {
                    bail!("unable to read acl config {:?} - {}", filename, err);
                }
            }
        };

        let digest = openssl::sha::sha256(raw.as_bytes());

        for (linenr, line) in raw.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() { continue; }
            if let Err(err) = tree.parse_acl_line(line) {
                bail!("unable to parse acl config {:?}, line {} - {}",
                      filename, linenr+1, err);
            }
        }

        Ok((tree, digest))
    }

    pub fn from_raw(raw: &str) -> Result<Self, Error> {
        let mut tree = Self::new();
        for (linenr, line) in raw.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() { continue; }
            if let Err(err) = tree.parse_acl_line(line) {
                bail!("unable to parse acl config data, line {} - {}", linenr+1, err);
            }
        }
        Ok(tree)
    }

    pub fn roles(&self, auth_id: &Authid, path: &[&str]) -> HashMap<String, bool> {

        let mut node = &self.root;
        let mut role_map = node.extract_roles(auth_id, path.is_empty());

        for (pos, comp) in path.iter().enumerate() {
            let last_comp = (pos + 1) == path.len();
            node = match node.children.get(*comp) {
                Some(n) => n,
                None => return role_map, // path not found
            };

            let new_map = node.extract_roles(auth_id, last_comp);
            if !new_map.is_empty() {
                // overwrite previous maptings
                role_map = new_map;
            }
        }

        role_map
    }
}

pub const ACL_CFG_FILENAME: &str = "/etc/proxmox-backup/acl.cfg";
pub const ACL_CFG_LOCKFILE: &str = "/etc/proxmox-backup/.acl.lck";

pub fn config() -> Result<(AclTree, [u8; 32]), Error> {
    let path = PathBuf::from(ACL_CFG_FILENAME);
    AclTree::load(&path)
}

pub fn cached_config() -> Result<Arc<AclTree>, Error> {

    struct ConfigCache {
        data: Option<Arc<AclTree>>,
        last_mtime: i64,
        last_mtime_nsec: i64,
    }

    lazy_static! {
        static ref CACHED_CONFIG: RwLock<ConfigCache> = RwLock::new(
            ConfigCache { data: None, last_mtime: 0, last_mtime_nsec: 0 });
    }

    let stat = match nix::sys::stat::stat(ACL_CFG_FILENAME) {
        Ok(stat) => Some(stat),
        Err(nix::Error::Sys(nix::errno::Errno::ENOENT)) => None,
        Err(err) => bail!("unable to stat '{}' - {}", ACL_CFG_FILENAME, err),
    };

    { // limit scope
        let cache = CACHED_CONFIG.read().unwrap();
        if let Some(ref config) = cache.data {
            if let Some(stat) = stat {
                if stat.st_mtime == cache.last_mtime && stat.st_mtime_nsec == cache.last_mtime_nsec {
                    return Ok(config.clone());
                }
            } else if cache.last_mtime == 0 && cache.last_mtime_nsec == 0 {
                return Ok(config.clone());
            }
        }
    }

    let (config, _digest) = config()?;
    let config = Arc::new(config);

    let mut cache = CACHED_CONFIG.write().unwrap();
    if let Some(stat) = stat {
        cache.last_mtime = stat.st_mtime;
        cache.last_mtime_nsec = stat.st_mtime_nsec;
    }
    cache.data = Some(config.clone());

    Ok(config)
}

pub fn save_config(acl: &AclTree) -> Result<(), Error> {
    let mut raw: Vec<u8> = Vec::new();

    acl.write_config(&mut raw)?;

    let backup_user = crate::backup::backup_user()?;
    let mode = nix::sys::stat::Mode::from_bits_truncate(0o0640);
    // set the correct owner/group/permissions while saving file
    // owner(rw) = root, group(r)= backup
    let options = CreateOptions::new()
        .perm(mode)
        .owner(nix::unistd::ROOT)
        .group(backup_user.gid);

    replace_file(ACL_CFG_FILENAME, &raw, options)?;

    Ok(())
}

#[cfg(test)]
mod test {
    use anyhow::{Error};
    use super::AclTree;

    use crate::api2::types::Authid;

    fn check_roles(
        tree: &AclTree,
        auth_id: &Authid,
        path: &str,
        expected_roles: &str,
    ) {

        let path_vec = super::split_acl_path(path);
        let mut roles = tree.roles(auth_id, &path_vec)
            .iter().map(|(v, _)| v.clone()).collect::<Vec<String>>();
        roles.sort();
        let roles = roles.join(",");

        assert_eq!(roles, expected_roles, "\nat check_roles for '{}' on '{}'", auth_id, path);
    }

    #[test]
    fn test_acl_line_compression() {

        let tree = AclTree::from_raw(
            "\
            acl:0:/store/store2:user1@pbs:Admin\n\
            acl:0:/store/store2:user2@pbs:Admin\n\
            acl:0:/store/store2:user1@pbs:DatastoreBackup\n\
            acl:0:/store/store2:user2@pbs:DatastoreBackup\n\
            ",
        )
        .expect("failed to parse acl tree");

        let mut raw: Vec<u8> = Vec::new();
        tree.write_config(&mut raw).expect("failed to write acl tree");
        let raw = std::str::from_utf8(&raw).expect("acl tree is not valid utf8");

        assert_eq!(raw, "acl:0:/store/store2:user1@pbs,user2@pbs:Admin,DatastoreBackup\n");
    }

    #[test]
    fn test_roles_1() -> Result<(), Error> {

        let tree = AclTree::from_raw(r###"
acl:1:/storage:user1@pbs:Admin
acl:1:/storage/store1:user1@pbs:DatastoreBackup
acl:1:/storage/store2:user2@pbs:DatastoreBackup
"###)?;
        let user1: Authid = "user1@pbs".parse()?;
        check_roles(&tree, &user1, "/", "");
        check_roles(&tree, &user1, "/storage", "Admin");
        check_roles(&tree, &user1, "/storage/store1", "DatastoreBackup");
        check_roles(&tree, &user1, "/storage/store2", "Admin");

        let user2: Authid = "user2@pbs".parse()?;
        check_roles(&tree, &user2, "/", "");
        check_roles(&tree, &user2, "/storage", "");
        check_roles(&tree, &user2, "/storage/store1", "");
        check_roles(&tree, &user2, "/storage/store2", "DatastoreBackup");

        Ok(())
    }

    #[test]
    fn test_role_no_access() -> Result<(), Error> {

        let tree = AclTree::from_raw(r###"
acl:1:/:user1@pbs:Admin
acl:1:/storage:user1@pbs:NoAccess
acl:1:/storage/store1:user1@pbs:DatastoreBackup
"###)?;
        let user1: Authid = "user1@pbs".parse()?;
        check_roles(&tree, &user1, "/", "Admin");
        check_roles(&tree, &user1, "/storage", "NoAccess");
        check_roles(&tree, &user1, "/storage/store1", "DatastoreBackup");
        check_roles(&tree, &user1, "/storage/store2", "NoAccess");
        check_roles(&tree, &user1, "/system", "Admin");

        let tree = AclTree::from_raw(r###"
acl:1:/:user1@pbs:Admin
acl:0:/storage:user1@pbs:NoAccess
acl:1:/storage/store1:user1@pbs:DatastoreBackup
"###)?;
        check_roles(&tree, &user1, "/", "Admin");
        check_roles(&tree, &user1, "/storage", "NoAccess");
        check_roles(&tree, &user1, "/storage/store1", "DatastoreBackup");
        check_roles(&tree, &user1, "/storage/store2", "Admin");
        check_roles(&tree, &user1, "/system", "Admin");

        Ok(())
    }

    #[test]
    fn test_role_add_delete() -> Result<(), Error> {

        let mut tree = AclTree::new();

        let user1: Authid = "user1@pbs".parse()?;

        tree.insert_user_role("/", &user1, "Admin", true);
        tree.insert_user_role("/", &user1, "Audit", true);

        check_roles(&tree, &user1, "/", "Admin,Audit");

        tree.insert_user_role("/", &user1, "NoAccess", true);
        check_roles(&tree, &user1, "/", "NoAccess");

        let mut raw: Vec<u8> = Vec::new();
        tree.write_config(&mut raw)?;
        let raw = std::str::from_utf8(&raw)?;

        assert_eq!(raw, "acl:1:/:user1@pbs:NoAccess\n");

        Ok(())
    }

    #[test]
    fn test_no_access_overwrite() -> Result<(), Error> {

        let mut tree = AclTree::new();

        let user1: Authid = "user1@pbs".parse()?;

        tree.insert_user_role("/storage", &user1, "NoAccess", true);

        check_roles(&tree, &user1, "/storage", "NoAccess");

        tree.insert_user_role("/storage", &user1, "Admin", true);
        tree.insert_user_role("/storage", &user1, "Audit", true);

        check_roles(&tree, &user1, "/storage", "Admin,Audit");

        tree.insert_user_role("/storage", &user1, "NoAccess", true);

        check_roles(&tree, &user1, "/storage", "NoAccess");

        Ok(())
    }
}
