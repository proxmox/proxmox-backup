use failure::*;

use crate::api::schema::*;
use crate::api::router::*;
use serde_json::{json, Value};

use std::sync::Arc;
use lazy_static::lazy_static;
use crate::tools::common_regex;
use std::process::{Command, Stdio};

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
	            if limit <= 0 { continue };

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
    rpcenv: &mut RpcEnvironment,
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

lazy_static! {
    pub static ref SYSTEMD_DATETIME_FORMAT: Arc<ApiStringFormat> =
        ApiStringFormat::Pattern(&common_regex::SYSTEMD_DATETIME_REGEX).into();
}

pub fn router() -> Router {

    let route = Router::new()
        .get(
            ApiMethod::new(
                get_syslog,
                ObjectSchema::new("Read server time and time zone settings.")
                    .optional(
                        "start",
                        IntegerSchema::new("Start line number.")
                            .minimum(0)
                    )
                    .optional(
                        "limit",
                        IntegerSchema::new("Max. number of lines.")
                            .minimum(0)
                    )
                    .optional(
                        "since",
                        StringSchema::new("Display all log since this date-time string.")
	                    .format(SYSTEMD_DATETIME_FORMAT.clone())
                    )
                    .optional(
                        "until",
                        StringSchema::new("Display all log until this date-time string.")
	                    .format(SYSTEMD_DATETIME_FORMAT.clone())
                    )
                    .optional(
                        "service",
                        StringSchema::new("Service ID.")
                            .max_length(128)
                    )
            ).returns(
                ObjectSchema::new("Returns a list of syslog entries.")
                    .required("n", IntegerSchema::new("Line number."))
                    .required("t", StringSchema::new("Line text."))
            ).protected(true)
        );

    route
}
