use std::process::{Command, Stdio};

use anyhow::Error;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader};

use proxmox_router::{ApiMethod, Permission, Router, RpcEnvironment};
use proxmox_schema::api;

use pbs_api_types::{NODE_SCHEMA, PRIV_SYS_AUDIT};

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
        permission: &Permission::Privilege(&["system", "log"], PRIV_SYS_AUDIT, false),
    },
)]
/// Read syslog entries.
#[allow(clippy::too_many_arguments)]
fn get_journal(
    since: Option<i64>,
    until: Option<i64>,
    lastentries: Option<u64>,
    startcursor: Option<String>,
    endcursor: Option<String>,
    _param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {
    let mut args = vec![];

    if let Some(lastentries) = lastentries {
        args.push(String::from("-n"));
        args.push(format!("{}", lastentries));
    }

    if let Some(since) = since {
        args.push(String::from("-b"));
        args.push(since.to_string());
    }

    if let Some(until) = until {
        args.push(String::from("-e"));
        args.push(until.to_string());
    }

    if let Some(startcursor) = startcursor {
        args.push(String::from("-f"));
        args.push(startcursor);
    }

    if let Some(endcursor) = endcursor {
        args.push(String::from("-t"));
        args.push(endcursor);
    }

    let mut lines: Vec<String> = vec![];

    let mut child = Command::new("mini-journalreader")
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

pub const ROUTER: Router = Router::new().get(&API_METHOD_GET_JOURNAL);
