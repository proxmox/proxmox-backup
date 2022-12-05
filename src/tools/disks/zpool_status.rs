use std::mem::{replace, take};

use anyhow::{bail, Error};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use pbs_tools::nom::{
    multispace0, multispace1, notspace1, parse_complete, parse_error, parse_failure, parse_u64,
    IResult,
};

use nom::{
    bytes::complete::{tag, take_while, take_while1},
    character::complete::line_ending,
    combinator::opt,
    multi::{many0, many1},
    sequence::preceded,
};

#[derive(Debug, Serialize, Deserialize)]
pub struct ZFSPoolVDevState {
    pub name: String,
    pub lvl: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub write: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cksum: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub msg: Option<String>,
}

fn expand_tab_length(input: &str) -> usize {
    input.chars().map(|c| if c == '\t' { 8 } else { 1 }).sum()
}

fn parse_zpool_status_vdev(i: &str) -> IResult<&str, ZFSPoolVDevState> {
    let (n, indent) = multispace0(i)?;

    let indent_len = expand_tab_length(indent);

    if (indent_len & 1) != 0 {
        return Err(parse_failure(n, "wrong indent length"));
    }
    let i = n;

    let indent_level = (indent_len as u64) / 2;

    let (i, vdev_name) = notspace1(i)?;

    if let Ok((n, _)) = preceded(multispace0, line_ending)(i) {
        // special device
        let vdev = ZFSPoolVDevState {
            name: vdev_name.to_string(),
            lvl: indent_level,
            state: None,
            read: None,
            write: None,
            cksum: None,
            msg: None,
        };
        return Ok((n, vdev));
    }

    let (i, state) = preceded(multispace1, notspace1)(i)?;
    if let Ok((n, _)) = preceded(multispace0, line_ending)(i) {
        // spares
        let vdev = ZFSPoolVDevState {
            name: vdev_name.to_string(),
            lvl: indent_level,
            state: Some(state.to_string()),
            read: None,
            write: None,
            cksum: None,
            msg: None,
        };
        return Ok((n, vdev));
    }

    let (i, read) = preceded(multispace1, parse_u64)(i)?;
    let (i, write) = preceded(multispace1, parse_u64)(i)?;
    let (i, cksum) = preceded(multispace1, parse_u64)(i)?;
    let (i, msg) = opt(preceded(multispace1, take_while(|c| c != '\n')))(i)?;
    let (i, _) = line_ending(i)?;

    let vdev = ZFSPoolVDevState {
        name: vdev_name.to_string(),
        lvl: indent_level,
        state: Some(state.to_string()),
        read: Some(read),
        write: Some(write),
        cksum: Some(cksum),
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

fn space_indented_line(indent: usize) -> impl Fn(&str) -> IResult<&str, &str> {
    move |i| {
        let mut len = 0;
        let mut n = i;
        loop {
            if n.starts_with('\t') {
                len += 8;
            } else if n.starts_with(' ') {
                len += 1;
            } else {
                break;
            }
            n = &n[1..];
            if len >= indent {
                break;
            }
        }
        if len != indent {
            return Err(parse_error(i, "not correctly indented"));
        }

        take_while1(|c| c != '\n')(n)
    }
}

fn parse_zpool_status_field(i: &str) -> IResult<&str, (String, String)> {
    let (i, prefix) = take_while1(|c| c != ':')(i)?;
    let (i, _) = tag(":")(i)?;
    let (i, mut value) = take_while(|c| c != '\n')(i)?;
    if value.starts_with(' ') {
        value = &value[1..];
    }

    let (mut i, _) = line_ending(i)?;

    let field = prefix.trim().to_string();

    let prefix_len = expand_tab_length(prefix);

    let indent: usize = prefix_len + 2;

    let mut parse_continuation = opt(space_indented_line(indent));

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
            if !value.is_empty() {
                value.push('\n');
            }
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

fn parse_zpool_status(input: &str) -> Result<Vec<(String, String)>, Error> {
    parse_complete("zfs status output", input, many0(parse_zpool_status_field))
}

pub fn vdev_list_to_tree(vdev_list: &[ZFSPoolVDevState]) -> Result<Value, Error> {
    indented_list_to_tree(vdev_list, |vdev| {
        let node = serde_json::to_value(vdev).unwrap();
        (node, vdev.lvl)
    })
}

fn indented_list_to_tree<'a, T, F, I>(items: I, to_node: F) -> Result<Value, Error>
where
    T: 'a,
    I: IntoIterator<Item = &'a T>,
    F: Fn(&T) -> (Value, u64),
{
    struct StackItem {
        node: Map<String, Value>,
        level: u64,
        children_of_parent: Vec<Value>,
    }

    let mut stack = Vec::<StackItem>::new();
    // hold current node and the children of the current parent (as that's where we insert)
    let mut cur = StackItem {
        node: Map::<String, Value>::new(),
        level: 0,
        children_of_parent: Vec::new(),
    };

    for item in items {
        let (node, node_level) = to_node(item);
        let vdev_level = 1 + node_level;
        let mut node = match node {
            Value::Object(map) => map,
            _ => bail!("to_node returned wrong type"),
        };

        node.insert("leaf".to_string(), Value::Bool(true));

        // if required, go back up (possibly multiple levels):
        while vdev_level < cur.level {
            cur.children_of_parent.push(Value::Object(cur.node));
            let mut parent = stack.pop().unwrap();
            parent
                .node
                .insert("children".to_string(), Value::Array(cur.children_of_parent));
            parent.node.insert("leaf".to_string(), Value::Bool(false));
            cur = parent;

            if vdev_level > cur.level {
                // when we encounter misimatching levels like "0, 2, 1" instead of "0, 1, 2, 1"
                bail!("broken indentation between levels");
            }
        }

        if vdev_level > cur.level {
            // indented further, push our current state and start a new "map"
            stack.push(StackItem {
                node: replace(&mut cur.node, node),
                level: replace(&mut cur.level, vdev_level),
                children_of_parent: take(&mut cur.children_of_parent),
            });
        } else {
            // same indentation level, add to children of the previous level:
            cur.children_of_parent
                .push(Value::Object(replace(&mut cur.node, node)));
        }
    }

    while !stack.is_empty() {
        cur.children_of_parent.push(Value::Object(cur.node));
        let mut parent = stack.pop().unwrap();
        parent
            .node
            .insert("children".to_string(), Value::Array(cur.children_of_parent));
        parent.node.insert("leaf".to_string(), Value::Bool(false));
        cur = parent;
    }

    Ok(Value::Object(cur.node))
}

#[test]
fn test_vdev_list_to_tree() {
    const DEFAULT: ZFSPoolVDevState = ZFSPoolVDevState {
        name: String::new(),
        lvl: 0,
        state: None,
        read: None,
        write: None,
        cksum: None,
        msg: None,
    };

    #[rustfmt::skip]
    let input = vec![
        //ZFSPoolVDevState { name: "root".to_string(), lvl: 0, ..DEFAULT },
        ZFSPoolVDevState { name: "vdev1".to_string(), lvl: 1, ..DEFAULT },
        ZFSPoolVDevState { name: "vdev1-disk1".to_string(), lvl: 2, ..DEFAULT },
        ZFSPoolVDevState { name: "vdev1-disk2".to_string(), lvl: 2, ..DEFAULT },
        ZFSPoolVDevState { name: "vdev2".to_string(), lvl: 1, ..DEFAULT },
        ZFSPoolVDevState { name: "vdev2-g1".to_string(), lvl: 2, ..DEFAULT },
        ZFSPoolVDevState { name: "vdev2-g1-d1".to_string(), lvl: 3, ..DEFAULT },
        ZFSPoolVDevState { name: "vdev2-g1-d2".to_string(), lvl: 3, ..DEFAULT },
        ZFSPoolVDevState { name: "vdev2-g2".to_string(), lvl: 2, ..DEFAULT },
        ZFSPoolVDevState { name: "vdev3".to_string(), lvl: 1, ..DEFAULT },
        ZFSPoolVDevState { name: "vdev4".to_string(), lvl: 1, ..DEFAULT },
        ZFSPoolVDevState { name: "vdev4-g1".to_string(), lvl: 2, ..DEFAULT },
        ZFSPoolVDevState { name: "vdev4-g1-d1".to_string(), lvl: 3, ..DEFAULT },
        ZFSPoolVDevState { name: "vdev4-g1-d1-x1".to_string(), lvl: 4, ..DEFAULT },
        ZFSPoolVDevState { name: "vdev4-g2".to_string(), lvl: 2, ..DEFAULT }, // up by 2
    ];

    const EXPECTED: &str = "{\
        \"children\":[{\
            \"children\":[{\
                \"leaf\":true,\
                \"lvl\":2,\"name\":\"vdev1-disk1\"\
            },{\
                \"leaf\":true,\
                \"lvl\":2,\"name\":\"vdev1-disk2\"\
            }],\
            \"leaf\":false,\
            \"lvl\":1,\"name\":\"vdev1\"\
        },{\
            \"children\":[{\
                \"children\":[{\
                    \"leaf\":true,\
                    \"lvl\":3,\"name\":\"vdev2-g1-d1\"\
                },{\
                    \"leaf\":true,\
                    \"lvl\":3,\"name\":\"vdev2-g1-d2\"\
                }],\
                \"leaf\":false,\
                \"lvl\":2,\"name\":\"vdev2-g1\"\
            },{\
                \"leaf\":true,\
                \"lvl\":2,\"name\":\"vdev2-g2\"\
            }],\
            \"leaf\":false,\
            \"lvl\":1,\"name\":\"vdev2\"\
        },{\
            \"leaf\":true,\
            \"lvl\":1,\"name\":\"vdev3\"\
        },{\
            \"children\":[{\
                \"children\":[{\
                    \"children\":[{\
                        \"leaf\":true,\
                        \"lvl\":4,\"name\":\"vdev4-g1-d1-x1\"\
                    }],\
                    \"leaf\":false,\
                    \"lvl\":3,\"name\":\"vdev4-g1-d1\"\
                }],\
                \"leaf\":false,\
                \"lvl\":2,\"name\":\"vdev4-g1\"\
            },{\
                \"leaf\":true,\
                \"lvl\":2,\"name\":\"vdev4-g2\"\
            }],\
            \"leaf\":false,\
            \"lvl\":1,\"name\":\"vdev4\"\
        }],\
        \"leaf\":false\
   }";
    let expected: Value =
        serde_json::from_str(EXPECTED).expect("failed to parse expected json value");

    let tree = vdev_list_to_tree(&input).expect("failed to turn valid vdev list into a tree");
    assert_eq!(tree, expected);
}

pub fn zpool_status(pool: &str) -> Result<Vec<(String, String)>, Error> {
    let mut command = std::process::Command::new("zpool");
    command.args(["status", "-p", "-P", pool]);

    let output = proxmox_sys::command::run_command(command, None)?;

    parse_zpool_status(&output)
}

#[cfg(test)]
fn test_parse(output: &str) -> Result<(), Error> {
    let mut found_config = false;

    for (k, v) in parse_zpool_status(output)? {
        println!("<{}> => '{}'", k, v);
        if k == "config" {
            let vdev_list = parse_zpool_status_config_tree(&v)?;
            let _tree = vdev_list_to_tree(&vdev_list);
            found_config = true;
        }
    }
    if !found_config {
        bail!("got zpool status without config key");
    }

    Ok(())
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

    test_parse(output)
}

#[test]
fn test_zpool_status_parser2() -> Result<(), Error> {
    // Note: this input create TABS
    let output = r###"  pool: btest
 state: ONLINE
  scan: none requested
config:

	NAME           STATE     READ WRITE CKSUM
	btest          ONLINE       0     0     0
	  mirror-0     ONLINE       0     0     0
	    /dev/sda1  ONLINE       0     0     0
	    /dev/sda2  ONLINE       0     0     0
	  mirror-1     ONLINE       0     0     0
	    /dev/sda3  ONLINE       0     0     0
	    /dev/sda4  ONLINE       0     0     0
	logs
	  /dev/sda5    ONLINE       0     0     0

errors: No known data errors
"###;
    test_parse(output)
}

#[test]
fn test_zpool_status_parser3() -> Result<(), Error> {
    let output = r###"  pool: bt-est
 state: ONLINE
  scan: none requested
config:

	NAME           STATE     READ WRITE CKSUM
	bt-est          ONLINE       0     0     0
	  mirror-0     ONLINE       0     0     0
	    /dev/sda1  ONLINE       0     0     0
	    /dev/sda2  ONLINE       0     0     0
	  mirror-1     ONLINE       0     0     0
	    /dev/sda3  ONLINE       0     0     0
	    /dev/sda4  ONLINE       0     0     0
	logs
	  /dev/sda5    ONLINE       0     0     0

errors: No known data errors
"###;

    test_parse(output)
}

#[test]
fn test_zpool_status_parser_spares() -> Result<(), Error> {
    let output = r###"  pool: tank
 state: ONLINE
  scan: none requested
config:

	NAME           STATE     READ WRITE CKSUM
	tank          ONLINE       0     0     0
	  mirror-0     ONLINE       0     0     0
	    /dev/sda1  ONLINE       0     0     0
	    /dev/sda2  ONLINE       0     0     0
	  mirror-1     ONLINE       0     0     0
	    /dev/sda3  ONLINE       0     0     0
	    /dev/sda4  ONLINE       0     0     0
	logs
	  /dev/sda5    ONLINE       0     0     0
        spares
          /dev/sdb     AVAIL
          /dev/sdc     AVAIL

errors: No known data errors
"###;

    test_parse(output)
}
