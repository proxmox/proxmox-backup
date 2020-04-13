use std::io::Write;
use std::collections::{HashMap, HashSet, BTreeMap, BTreeSet};
use std::path::{PathBuf, Path};

use failure::*;

use lazy_static::lazy_static;

use proxmox::tools::{fs::replace_file, fs::CreateOptions};

// define Privilege bitfield

pub const PRIV_SYS_AUDIT: u64               = 1 << 0;
pub const PRIV_SYS_MODIFY: u64              = 1 << 1;
pub const PRIV_SYS_POWER_MANAGEMENT: u64    = 1 << 2;

pub const PRIV_STORE_AUDIT: u64              = 1 << 3;
pub const PRIV_STORE_ALLOCATE: u64           = 1 << 4;
pub const PRIV_STORE_ALLOCATE_SPACE: u64     = 1 << 5;

pub const ROLE_ADMIN: u64 = std::u64::MAX;
pub const ROLE_NO_ACCESS: u64 = 0;

pub const ROLE_AUDIT: u64 =
PRIV_SYS_AUDIT |
PRIV_STORE_AUDIT;

pub const ROLE_STORE_ADMIN: u64 =
PRIV_STORE_AUDIT |
PRIV_STORE_ALLOCATE |
PRIV_STORE_ALLOCATE_SPACE;

pub const ROLE_STORE_USER: u64 =
PRIV_STORE_AUDIT |
PRIV_STORE_ALLOCATE_SPACE;

lazy_static! {
    static ref ROLE_NAMES: HashMap<&'static str, u64> = {
        let mut map = HashMap::new();

        map.insert("Admin", ROLE_ADMIN);
        map.insert("Audit", ROLE_AUDIT);
        map.insert("NoAccess", ROLE_NO_ACCESS);

        map.insert("Store.Admin", ROLE_STORE_ADMIN);
        map.insert("Store.User", ROLE_STORE_USER);

        map
    };
}

fn split_acl_path(path: &str) -> Vec<&str> {

    let items = path.split('/');

    let mut components = vec![];

    for name in items {
        if name.is_empty() { continue; }
        components.push(name);
    }

    components
}

pub struct AclTree {
    pub root: AclTreeNode,
}

pub struct AclTreeNode {
    pub users: HashMap<String, HashMap<String, bool>>,
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

    pub fn extract_roles(&self, user: &str, all: bool) -> HashSet<String> {
        let user_roles = self.extract_user_roles(user, all);
        if !user_roles.is_empty() {
            // user privs always override group privs
            return user_roles
        };

        self.extract_group_roles(user, all)
    }

    pub fn extract_user_roles(&self, user: &str, all: bool) -> HashSet<String> {

        let mut set = HashSet::new();

        let roles = match self.users.get(user) {
            Some(m) => m,
            None => return set,
        };

        for (role, propagate) in roles {
            if *propagate || all {
                if role == "NoAccess" {
                    // return a set with a single role 'NoAccess'
                    let mut set = HashSet::new();
                    set.insert(role.to_string());
                    return set;
                }
                set.insert(role.to_string());
            }
        }

        set
    }

    pub fn extract_group_roles(&self, _user: &str, all: bool) -> HashSet<String> {

        let mut set = HashSet::new();

        for (_group, roles) in &self.groups {
            let is_member = false; // fixme: check if user is member of the group
            if !is_member { continue; }

            for (role, propagate) in roles {
                if *propagate || all {
                    if role == "NoAccess" {
                        // return a set with a single role 'NoAccess'
                        let mut set = HashSet::new();
                        set.insert(role.to_string());
                        return set;
                    }
                    set.insert(role.to_string());
                }
            }
        }

        set
    }

    pub fn insert_group_role(&mut self, group: String, role: String, propagate: bool) {
        self.groups
            .entry(group).or_insert_with(|| HashMap::new())
            .insert(role, propagate);
    }

    pub fn insert_user_role(&mut self, user: String, role: String, propagate: bool) {
        self.users
            .entry(user).or_insert_with(|| HashMap::new())
            .insert(role, propagate);
    }
}

impl AclTree {

    pub fn new() -> Self {
        Self { root: AclTreeNode::new() }
    }

    fn get_or_insert_node(&mut self, path: &[&str]) -> &mut AclTreeNode {
        let mut node = &mut self.root;
        for comp in path {
            node = node.children.entry(String::from(*comp))
                .or_insert_with(|| AclTreeNode::new());
        }
        node
    }

    pub fn insert_group_role(&mut self, path: &str, group: &str, role: &str, propagate: bool) {
        let path = split_acl_path(path);
        let node = self.get_or_insert_node(&path);
        node.insert_group_role(group.to_string(), role.to_string(), propagate);
    }

    pub fn insert_user_role(&mut self, path: &str, user: &str, role: &str, propagate: bool) {
        let path = split_acl_path(path);
        let node = self.get_or_insert_node(&path);
        node.insert_user_role(user.to_string(), role.to_string(), propagate);
    }

    fn write_node_config(
        node: &AclTreeNode,
        path: &str,
        w: &mut dyn Write,
    ) -> Result<(), Error> {

        let mut role_ug_map0 = HashMap::new();
        let mut role_ug_map1 = HashMap::new();

        for (user, roles) in &node.users {
            // no need to save, because root is always 'Administrator'
            if user == "root@pam" { continue; }
            for (role, propagate) in roles {
                let role = role.as_str();
                let user = user.to_string();
                if *propagate {
                    role_ug_map1.entry(role).or_insert_with(|| BTreeSet::new())
                        .insert(user);
                } else {
                    role_ug_map0.entry(role).or_insert_with(|| BTreeSet::new())
                        .insert(user);
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

        for (uglist, roles) in uglist_role_map0 {
            let role_list = roles.iter().fold(String::new(), |mut acc, v| {
                if !acc.is_empty() { acc.push(','); }
                acc.push_str(v);
                acc
            });
            writeln!(w, "acl:0:{}:{}:{}", path, uglist, role_list)?;
        }

        for (uglist, roles) in uglist_role_map1 {
           let role_list = roles.iter().fold(String::new(), |mut acc, v| {
                if !acc.is_empty() { acc.push(','); }
                acc.push_str(v);
                acc
            });
            writeln!(w, "acl:1:{}:{}:{}", path, uglist, role_list)?;
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

        let path = split_acl_path(items[2]);
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
                    node.insert_user_role(user_or_group.to_string(), role.to_string(), propagate);
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

    pub fn roles(&self, userid: &str, path: &[&str]) -> HashSet<String> {

        let mut node = &self.root;
        let mut role_set = node.extract_roles(userid, path.is_empty());

        for (pos, comp) in path.iter().enumerate() {
            let last_comp = (pos + 1) == path.len();
            node = match node.children.get(*comp) {
                Some(n) => n,
                None => return role_set, // path not found
            };
            let new_set = node.extract_roles(userid, last_comp);
            if !new_set.is_empty() {
                // overwrite previous settings
                role_set = new_set;
            }
        }

        role_set
    }
}

pub const ACL_CFG_FILENAME: &str = "/etc/proxmox-backup/acl.cfg";
pub const ACL_CFG_LOCKFILE: &str = "/etc/proxmox-backup/.acl.lck";

pub fn config() -> Result<(AclTree, [u8; 32]), Error> {
    let path = PathBuf::from(ACL_CFG_FILENAME);
    AclTree::load(&path)
}

pub fn store_config(acl: &AclTree, filename: &Path) -> Result<(), Error> {
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

    replace_file(filename, &raw, options)?;

    Ok(())
}


#[cfg(test)]
mod test {

    use failure::*;
    use super::AclTree;

    fn check_roles(
        tree: &AclTree,
        user: &str,
        path: &str,
        expected_roles: &str,
    ) {

        let path_vec = super::split_acl_path(path);
        let mut roles = tree.roles(user, &path_vec)
            .iter().map(|v| v.clone()).collect::<Vec<String>>();
        roles.sort();
        let roles = roles.join(",");

        assert_eq!(roles, expected_roles, "\nat check_roles for '{}' on '{}'", user, path);
    }

    #[test]
    fn test_acl_line_compression() -> Result<(), Error> {

        let tree = AclTree::from_raw(r###"
acl:0:/store/store2:user1:Admin
acl:0:/store/store2:user2:Admin
acl:0:/store/store2:user1:Store.User
acl:0:/store/store2:user2:Store.User
"###)?;

        let mut raw: Vec<u8> = Vec::new();
        tree.write_config(&mut raw)?;
        let raw = std::str::from_utf8(&raw)?;

        assert_eq!(raw, "acl:0:/store/store2:user1,user2:Admin,Store.User\n");

        Ok(())
    }

    #[test]
    fn test_roles_1() -> Result<(), Error> {

        let tree = AclTree::from_raw(r###"
acl:1:/storage:user1@pbs:Admin
acl:1:/storage/store1:user1@pbs:Store.User
acl:1:/storage/store2:user2@pbs:Store.User
"###)?;
        check_roles(&tree, "user1@pbs", "/", "");
        check_roles(&tree, "user1@pbs", "/storage", "Admin");
        check_roles(&tree, "user1@pbs", "/storage/store1", "Store.User");
        check_roles(&tree, "user1@pbs", "/storage/store2", "Admin");

        check_roles(&tree, "user2@pbs", "/", "");
        check_roles(&tree, "user2@pbs", "/storage", "");
        check_roles(&tree, "user2@pbs", "/storage/store1", "");
        check_roles(&tree, "user2@pbs", "/storage/store2", "Store.User");

        Ok(())
    }

    #[test]
    fn test_role_no_access() -> Result<(), Error> {

        let tree = AclTree::from_raw(r###"
acl:1:/:user1@pbs:Admin
acl:1:/storage:user1@pbs:NoAccess
acl:1:/storage/store1:user1@pbs:Store.User
"###)?;
        check_roles(&tree, "user1@pbs", "/", "Admin");
        check_roles(&tree, "user1@pbs", "/storage", "NoAccess");
        check_roles(&tree, "user1@pbs", "/storage/store1", "Store.User");
        check_roles(&tree, "user1@pbs", "/storage/store2", "NoAccess");
        check_roles(&tree, "user1@pbs", "/system", "Admin");

        let tree = AclTree::from_raw(r###"
acl:1:/:user1@pbs:Admin
acl:0:/storage:user1@pbs:NoAccess
acl:1:/storage/store1:user1@pbs:Store.User
"###)?;
        check_roles(&tree, "user1@pbs", "/", "Admin");
        check_roles(&tree, "user1@pbs", "/storage", "NoAccess");
        check_roles(&tree, "user1@pbs", "/storage/store1", "Store.User");
        check_roles(&tree, "user1@pbs", "/storage/store2", "Admin");
        check_roles(&tree, "user1@pbs", "/system", "Admin");

        Ok(())
    }
}
