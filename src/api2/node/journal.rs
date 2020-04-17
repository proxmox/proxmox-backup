use std::process::{Command, Stdio};

use anyhow::{Error};
use serde_json::{json, Value};
use std::io::{BufRead,BufReader};

use proxmox::api::{api, ApiMethod, Router, RpcEnvironment, Permission};

use crate::api2::types::*;
use crate::config::acl::PRIV_SYS_AUDIT;

#[api(
    protected: true,
    input: {
        properties: {
            node: {
                schema: NODE_SCHEMA,
            },
            since: {
                type: Integer,
                optional: true,
                description: "Display all log since this UNIX epoch. Conflicts with 'startcursor'.",
                minimum: 0,
            },
            until: {
                type: Integer,
                optional: true,
                description: "Display all log until this UNIX epoch. Conflicts with 'endcursor'.",
                minimum: 0,
            },
            lastentries: {
                type: Integer,
                optional: true,
                description: "Limit to the last X lines. Conflicts with a range.",
                minimum: 0,
            },
            startcursor: {
                type: String,
                description: "Start after the given Cursor. Conflicts with 'since'.",
                optional: true,
            },
            endcursor: {
                type: String,
                description: "End before the given Cursor. Conflicts with 'until'",
                optional: true,
            },
        },
    },
    returns: {
        type: Array,
        description: "Returns a list of journal entries.",
        items: {
            type: String,
            description: "Line text.",
        },
    },
    access: {
        permission: &Permission::Privilege(&[], PRIV_SYS_AUDIT, false),
    },
)]
/// Read syslog entries.
fn get_journal(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let mut args = vec![];

    if let Some(lastentries) = param["lastentries"].as_u64() {
        args.push(String::from("-n"));
        args.push(format!("{}", lastentries));
    }

    if let Some(since) = param["since"].as_str() {
        args.push(String::from("-b"));
        args.push(since.to_owned());
    }

    if let Some(until) = param["until"].as_str() {
        args.push(String::from("-e"));
        args.push(until.to_owned());
    }

    if let Some(startcursor) = param["startcursor"].as_str() {
        args.push(String::from("-f"));
        args.push(startcursor.to_owned());
    }

    if let Some(endcursor) = param["endcursor"].as_str() {
        args.push(String::from("-t"));
        args.push(endcursor.to_owned());
    }

    let mut lines: Vec<String> = vec![];

    let mut child = Command::new("/usr/bin/mini-journalreader")
        .args(&args)
        .stdout(Stdio::piped())
        .spawn()?;

    if let Some(ref mut stdout) = child.stdout {
        for line in BufReader::new(stdout).lines() {
            match line {
                Ok(line) => {
                    lines.push(line);
                }
                Err(err) => {
                    log::error!("reading journal failed: {}", err);
                    let _ = child.kill();
                    break;
                }
            }
        }
    }

    let status = child.wait().unwrap();
    if !status.success() {
        log::error!("journalctl failed with {}", status);
    }

    Ok(json!(lines))
}

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_GET_JOURNAL);
