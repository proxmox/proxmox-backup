use std::io::Write;
use std::collections::{HashMap, HashSet};
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

        map.insert("Admin", std::u64::MAX);
        map.insert("Audit", ROLE_AUDIT);


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
    root: AclTreeNode,
}

struct AclTreeNode {
    users: HashMap<String, HashMap<String, bool>>,
    groups: HashMap<String, HashMap<String, bool>>,
    children: HashMap<String, AclTreeNode>,
}

impl AclTreeNode {

    pub fn new() -> Self {
        Self {
            users: HashMap::new(),
            groups: HashMap::new(),
            children: HashMap::new(),
        }
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
                    role_ug_map1.entry(role).or_insert_with(|| HashSet::new())
                        .insert(user);
                } else {
                    role_ug_map0.entry(role).or_insert_with(|| HashSet::new())
                        .insert(user);
                }
            }
        }

        for (group, roles) in &node.groups {
            for (role, propagate) in roles {
                let group = format!("@{}", group);
                if *propagate {
                    role_ug_map1.entry(role).or_insert_with(|| HashSet::new())
                        .insert(group);
                } else {
                    role_ug_map0.entry(role).or_insert_with(|| HashSet::new())
                        .insert(group);
                }
            }
        }

        fn group_by_property_list(
            item_property_map: &HashMap<&str, HashSet<String>>,
        ) -> HashMap<String, HashSet<String>> {
            let mut result_map = HashMap::new();
            for (item, property_map) in item_property_map {
                let mut item_list = property_map.iter().map(|v| v.as_str())
                    .collect::<Vec<&str>>();
                item_list.sort();
                let item_list = item_list.join(",");
                result_map.entry(item_list).or_insert_with(|| HashSet::new())
                    .insert(item.to_string());
            }
            result_map
        }

        let mut uglist_role_map0 = group_by_property_list(&role_ug_map0)
            .into_iter()
            .collect::<Vec<(String, HashSet<String>)>>();
        uglist_role_map0.sort_unstable_by(|a,b| a.0.cmp(&b.0));

        let mut uglist_role_map1 = group_by_property_list(&role_ug_map1)
            .into_iter()
            .collect::<Vec<(String, HashSet<String>)>>();
        uglist_role_map1.sort_unstable_by(|a,b| a.0.cmp(&b.0));


        for (uglist, roles) in uglist_role_map0 {
            let mut role_list = roles.iter().map(|v| v.as_str())
                .collect::<Vec<&str>>();
            role_list.sort();
            writeln!(w, "acl:0:{}:{}:{}", path, uglist, role_list.join(","))?;
        }

        for (uglist, roles) in uglist_role_map1 {
            let mut role_list = roles.iter().map(|v| v.as_str())
                .collect::<Vec<&str>>();
            role_list.sort();
            writeln!(w, "acl:1:{}:{}:{}", path, uglist, role_list.join(","))?;
        }

        let mut child_names = node.children.keys().map(|v| v.as_str()).collect::<Vec<&str>>();
        child_names.sort();

        for name in child_names {
            let child = node.children.get(name).unwrap();
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
            if let Err(err) = tree.parse_acl_line(line) {
                bail!("unable to parse acl config {:?}, line {} - {}", filename, linenr, err);
            }
        }

        Ok((tree, digest))
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
