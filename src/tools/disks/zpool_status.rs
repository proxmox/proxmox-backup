use anyhow::{Error};
use serde_json::{json, Value};
use ::serde::{Deserialize, Serialize};

use crate::tools::nom::{
    parse_complete, parse_failure, multispace0, multispace1, notspace1, parse_u64, IResult,
};

use nom::{
    bytes::complete::{tag, take_while, take_while1},
    combinator::{opt},
    sequence::{preceded},
    character::complete::{line_ending},
    multi::{many0,many1},
};


#[derive(Debug, Serialize, Deserialize)]
pub struct ZFSPoolVDevState {
    pub name: String,
    pub lvl: u64,
    pub state: String,
    pub read: u64,
    pub write: u64,
    pub cksum: u64,
    #[serde(skip_serializing_if="Option::is_none")]
    pub msg: Option<String>,
}

fn parse_zpool_status_vdev(i: &str) -> IResult<&str, ZFSPoolVDevState> {

    let (n, indent) = multispace0(i)?;
    if (indent.len() & 1) != 0 {
        return Err(parse_failure(n, "wrong indent length"));
    }
    let i = n;

    let (i, vdev_name) =  notspace1(i)?;
    let (i, state) = preceded(multispace1, notspace1)(i)?;
    let (i, read) = preceded(multispace1, parse_u64)(i)?;
    let (i, write) = preceded(multispace1, parse_u64)(i)?;
    let (i, cksum) = preceded(multispace1, parse_u64)(i)?;
    let (i, msg) = opt(preceded(multispace1, take_while(|c| c != '\n')))(i)?;
    let (i, _) = line_ending(i)?;

    let vdev = ZFSPoolVDevState {
        name: vdev_name.to_string(),
        lvl: (indent.len() as u64)/2,
        state: state.to_string(),
        read, write, cksum,
        msg: msg.map(String::from),
    };

    Ok((i, vdev))
}

fn parse_zpool_status_tree(i: &str) -> IResult<&str, Vec<ZFSPoolVDevState>> {

    // skip header
    let (i, _) = tag("NAME")(i)?;
    let (i, _) = multispace1(i)?;
    let (i, _) = tag("STATE")(i)?;
    let (i, _) = multispace1(i)?;
    let (i, _) = tag("READ")(i)?;
    let (i, _) = multispace1(i)?;
    let (i, _) = tag("WRITE")(i)?;
    let (i, _) = multispace1(i)?;
    let (i, _) = tag("CKSUM")(i)?;
    let (i, _) = line_ending(i)?;

    // parse vdev list
    many1(parse_zpool_status_vdev)(i)
}

fn parse_zpool_status_field(i: &str) -> IResult<&str, (String, String)> {
    let (i, prefix) = take_while1(|c| c != ':')(i)?;
    let (i, _) = tag(":")(i)?;
    let (i, mut value) = take_while(|c| c != '\n')(i)?;
    if value.starts_with(' ') { value = &value[1..]; }

    let (mut i, _) = line_ending(i)?;

    let field = prefix.trim().to_string();

    let indent = (0..prefix.len()+2).fold(String::new(), |mut acc, _| { acc.push(' '); acc });

    let parse_continuation = opt(preceded(tag(indent.as_str()), take_while1(|c| c != '\n')));

    let mut value = value.to_string();

    if field == "config" {
        let (n, _) = line_ending(i)?;
        i = n;
    }

    loop {
        let (n, cont) = parse_continuation(i)?;

        if let Some(cont) = cont {
            let (n, _) = line_ending(n)?;
            i = n;
            if !value.is_empty() { value.push('\n'); }
            value.push_str(cont);
        } else {
            if field == "config" {
                let (n, _) = line_ending(i)?;
                value.push('\n');
                i = n;
            }
            break;
        }
    }

    Ok((i, (field, value)))
}

pub fn parse_zpool_status_config_tree(i: &str) -> Result<Vec<ZFSPoolVDevState>, Error> {
    parse_complete("zfs status config tree", i, parse_zpool_status_tree)
}

fn parse_zpool_status(i: &str) -> Result<Vec<(String, String)>, Error> {
    parse_complete("zfs status output", i, many0(parse_zpool_status_field))
}

pub fn vdev_list_to_tree(vdev_list: &[ZFSPoolVDevState]) -> Value {

    #[derive(Debug)]
    struct TreeNode<'a> {
        vdev: &'a ZFSPoolVDevState,
        children: Vec<usize>
    }

    fn node_to_json(node_idx: usize, nodes: &[TreeNode]) -> Value {
        let node = &nodes[node_idx];
        let mut v = serde_json::to_value(node.vdev).unwrap();
        if node.children.is_empty() {
            v["leaf"] = true.into();
        } else {
            v["leaf"] = false.into();
            v["children"] = json!([]);
            for child in node.children .iter(){
                let c = node_to_json(*child, nodes);
                v["children"].as_array_mut().unwrap().push(c);
            }
        }
        v
    }

    let mut nodes: Vec<TreeNode> = vdev_list.into_iter().map(|vdev| {
        TreeNode {
            vdev: vdev,
            children: Vec::new(),
        }
    }).collect();

    let mut stack: Vec<usize> = Vec::new();

    let mut root_children: Vec<usize> = Vec::new();

    for idx in 0..nodes.len() {

        if stack.is_empty() {
            root_children.push(idx);
            stack.push(idx);
            continue;
        }

        let node_lvl = nodes[idx].vdev.lvl;

        let stacked_node = &mut nodes[*(stack.last().unwrap())];
        let last_lvl = stacked_node.vdev.lvl;

        if node_lvl > last_lvl {
            stacked_node.children.push(idx);
        } else if node_lvl == last_lvl {
            stack.pop();
            match stack.last() {
                Some(parent) => nodes[*parent].children.push(idx),
                None => root_children.push(idx),
            }
        } else {
            loop {
                if stack.is_empty() {
                    root_children.push(idx);
                    break;
                }

                let stacked_node = &mut nodes[*(stack.last().unwrap())];
                if node_lvl <= stacked_node.vdev.lvl {
                    stack.pop();
                } else {
                    stacked_node.children.push(idx);
                    break;
                }
            }
        }

        stack.push(idx);
    }

    let mut result = json!({
        "name": "root",
        "children": json!([]),
    });

    for child in root_children {
        let c = node_to_json(child, &nodes);
        result["children"].as_array_mut().unwrap().push(c);
    }

    result
}

pub fn zpool_status(pool: &str) -> Result<Vec<(String, String)>, Error> {

    let mut command = std::process::Command::new("zpool");
    command.args(&["status", "-p", "-P", pool]);

    let output = crate::tools::run_command(command, None)?;

    parse_zpool_status(&output)
}

#[test]
fn test_zpool_status_parser() -> Result<(), Error> {

    let output = r###"  pool: tank
 state: DEGRADED
status: One or more devices could not be opened.  Sufficient replicas exist for
        the pool to continue functioning in a degraded state.
action: Attach the missing device and online it using 'zpool online'.
   see: http://www.sun.com/msg/ZFS-8000-2Q
 scrub: none requested
config:

        NAME        STATE     READ WRITE CKSUM
        tank        DEGRADED     0     0     0
         mirror-0  DEGRADED     0     0     0
            c1t0d0  ONLINE       0     0     0
            c1t2d0  ONLINE       0     0     0
            c1t1d0  UNAVAIL      0     0     0  cannot open
          mirror-1  DEGRADED     0     0     0
        tank1       DEGRADED     0     0     0
        tank2       DEGRADED     0     0     0

errors: No known data errors
"###;

    let key_value_list = parse_zpool_status(&output)?;
    for (k, v) in key_value_list {
        println!("{} => {}", k,v);
        if k == "config" {
            let vdev_list = parse_zpool_status_config_tree(&v)?;
            let tree = vdev_list_to_tree(&vdev_list);
            println!("TREE1 {}", serde_json::to_string_pretty(&tree)?);
        }
    }

    Ok(())
}
