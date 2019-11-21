use std::process::{Command, Stdio};

use failure::*;
use serde_json::{json, Value};

use proxmox::{sortable, identity};
use proxmox::api::{ApiHandler, ApiMethod, Router, RpcEnvironment};
use proxmox::api::schema::*;

use crate::api2::types::*;

fn dump_journal(
    start: Option<u64>,
    limit: Option<u64>,
    since: Option<&str>,
    until: Option<&str>,
    service: Option<&str>,
) -> Result<(u64, Vec<Value>), Error> {

    let mut args = vec!["-o", "short", "--no-pager"];

    if let Some(service) = service { args.extend(&["--unit", service]); }
    if let Some(since) = since { args.extend(&["--since", since]); }
    if let Some(until) = until { args.extend(&["--until", until]); }

    let mut lines: Vec<Value> = vec![];
    let mut limit = limit.unwrap_or(50);
    let start = start.unwrap_or(0);
    let mut count: u64 = 0;

    let mut child = Command::new("/bin/journalctl")
        .args(&args)
        .stdout(Stdio::piped())
        .spawn()?;

    use std::io::{BufRead,BufReader};

    if let Some(ref mut stdout) = child.stdout {
        for line in BufReader::new(stdout).lines() {
            match line {
                Ok(line) => {
                    count += 1;
                    if count < start { continue };
	            if limit == 0 { continue };

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

fn get_syslog(
    param: Value,
    _info: &ApiMethod,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let (count, lines) = dump_journal(
        param["start"].as_u64(),
        param["limit"].as_u64(),
        param["since"].as_str(),
        param["until"].as_str(),
        param["service"].as_str())?;

    rpcenv.set_result_attrib("total", Value::from(count));

    Ok(json!(lines))
}

#[sortable]
pub const ROUTER: Router = Router::new()
    .get(
        &ApiMethod::new(
            &ApiHandler::Sync(&get_syslog),
            &ObjectSchema::new(
                "Read server time and time zone settings.",
                &sorted!([
                    ("node", false, &NODE_SCHEMA),
                    ("start", true, &IntegerSchema::new("Start line number.")
                     .minimum(0)
                     .schema()
                    ),
                    ("limit", true, &IntegerSchema::new("Max. number of lines.")
                     .minimum(0)
                     .schema()
                    ),
                    ("since", true, &StringSchema::new("Display all log since this date-time string.")
	             .format(&SYSTEMD_DATETIME_FORMAT)
                     .schema()
                    ),
                    ("until", true, &StringSchema::new("Display all log until this date-time string.")
	             .format(&SYSTEMD_DATETIME_FORMAT)
                     .schema()
                    ),
                    ("service", true, &StringSchema::new("Service ID.")
                     .max_length(128)
                     .schema()
                    ),
                ]),
            )
        ).returns(
            &ObjectSchema::new(
                "Returns a list of syslog entries.",
                &sorted!([
                    ("n", false, &IntegerSchema::new("Line number.").schema()),
                    ("t", false, &StringSchema::new("Line text.").schema()),
                ]),
            ).schema()
        ).protected(true)
    );

