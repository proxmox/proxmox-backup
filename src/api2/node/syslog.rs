use std::process::{Command, Stdio};

use anyhow::Error;
use serde_json::{json, Value};

use proxmox_router::{ApiMethod, Permission, Router, RpcEnvironment};
use proxmox_schema::api;

use pbs_api_types::{NODE_SCHEMA, PRIV_SYS_AUDIT, SYSTEMD_DATETIME_FORMAT};

fn dump_journal(
    start: Option<u64>,
    limit: Option<u64>,
    since: Option<&str>,
    until: Option<&str>,
    service: Option<&str>,
) -> Result<(u64, Vec<Value>), Error> {
    let mut args = vec!["-o", "short", "--no-pager"];

    if let Some(service) = service {
        args.extend(["--unit", service]);
    }
    if let Some(since) = since {
        args.extend(["--since", since]);
    }
    if let Some(until) = until {
        args.extend(["--until", until]);
    }

    let mut lines: Vec<Value> = vec![];
    let mut limit = limit.unwrap_or(50);
    let start = start.unwrap_or(0);
    let mut count: u64 = 0;

    let mut child = Command::new("journalctl")
        .args(&args)
        .stdout(Stdio::piped())
        .spawn()?;

    use std::io::{BufRead, BufReader};

    if let Some(ref mut stdout) = child.stdout {
        for line in BufReader::new(stdout).lines() {
            match line {
                Ok(line) => {
                    count += 1;
                    if count < start {
                        continue;
                    };
                    if limit == 0 {
                        continue;
                    };

                    lines.push(json!({ "n": count, "t": line }));

                    limit -= 1;
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

    // HACK: ExtJS store.guaranteeRange() does not like empty array
    // so we add a line
    if count == 0 {
        count += 1;
        lines.push(json!({ "n": count, "t": "no content"}));
    }

    Ok((count, lines))
}

#[api(
    protected: true,
    input: {
        properties: {
            node: {
                schema: NODE_SCHEMA,
            },
            start: {
                type: Integer,
                description: "Start line number.",
                minimum: 0,
                optional: true,
            },
            limit: {
                type: Integer,
                description: "Max. number of lines.",
                optional: true,
                minimum: 0,
            },
            since: {
                type: String,
                optional: true,
                description: "Display all log since this date-time string.",
	        format: &SYSTEMD_DATETIME_FORMAT,
            },
            until: {
                type: String,
                optional: true,
                description: "Display all log until this date-time string.",
	        format: &SYSTEMD_DATETIME_FORMAT,
            },
            service: {
                type: String,
                optional: true,
                description: "Service ID.",
                max_length: 128,
            },
        },
    },
    returns: {
        type: Object,
        description: "Returns a list of syslog entries.",
        properties: {
            n: {
                type: Integer,
                description: "Line number.",
            },
            t: {
                type: String,
                description: "Line text.",
            }
        },
    },
    access: {
        permission: &Permission::Privilege(&["system", "log"], PRIV_SYS_AUDIT, false),
    },
)]
/// Read syslog entries.
fn get_syslog(
    param: Value,
    _info: &ApiMethod,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {
    let service = param["service"]
        .as_str()
        .map(crate::api2::node::services::real_service_name);

    let (count, lines) = dump_journal(
        param["start"].as_u64(),
        param["limit"].as_u64(),
        param["since"].as_str(),
        param["until"].as_str(),
        service,
    )?;

    rpcenv["total"] = Value::from(count);

    Ok(json!(lines))
}

pub const ROUTER: Router = Router::new().get(&API_METHOD_GET_SYSLOG);
