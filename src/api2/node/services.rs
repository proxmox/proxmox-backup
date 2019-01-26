use failure::*;

use crate::tools;
use crate::api::schema::*;
use crate::api::router::*;
use serde_json::{json, Value};

use std::sync::Arc;
use lazy_static::lazy_static;
use crate::tools::common_regex;
use std::process::{Command, Stdio};

static SERVICE_NAME_LIST: [&str; 6] = [
    "proxmox-backup",
    "sshd",
    "syslog",
    "cron",
    "postfix",
    "systemd-timesyncd",
];

fn get_full_service_state(service: &str) -> Result<Value, Error> {

    let mut real_service_name = service;

    // since postfix package 3.1.0-3.1 the postfix unit is only here
    // to manage subinstances, of which the default is called "-".
    // This is where we look for the daemon status

    if service == "postfix" { real_service_name = "postfix@-"; }

    let mut child = Command::new("/bin/systemctl")
        .args(&["show", real_service_name])
        .stdout(Stdio::piped())
        .spawn()?;

    use std::io::{BufRead,BufReader};

    let mut result = json!({});

    if let Some(ref mut stdout) = child.stdout {
        for line in BufReader::new(stdout).lines() {
            match line {
                Ok(line) => {
                    let mut iter = line.splitn(2, '=');
                    let key = iter.next();
                    let value = iter.next();
                    if let (Some(key), Some(value)) = (key, value) {
                        result[key] = Value::from(value);
                    }
                }
                Err(err) => {
                    log::error!("reading service config failed: {}", err);
                    let _ = child.kill();
                    break;
                }
            }
        }
    }

    let status = child.wait().unwrap();
    if !status.success() {
        bail!("systemctl show failed with {}", status);
    }

    Ok(result)
}

fn list_services(
    param: Value,
    _info: &ApiMethod,
    rpcenv: &mut RpcEnvironment,
) -> Result<Value, Error> {

    let mut list = vec![];

    for service in &SERVICE_NAME_LIST {
        match get_full_service_state(service) {
            Ok(status) => {
                if let Some(desc) = status["Description"].as_str() {
                    let name = status["Name"].as_str().unwrap_or(service);
                    let state = status["SubState"].as_str().unwrap_or("unknown");
                    list.push(json!({
                        "service": service,
                        "name": name,
                        "desc": desc,
                        "state": state,
                    }));
                }
            }
            Err(err) => log::error!("{}", err),
        }
    }

    Ok(Value::from(list))
}

pub fn router() -> Router {

    let route = Router::new()
        .get(
            ApiMethod::new(
                list_services,
                ObjectSchema::new("Service list.")
            ).returns(
                ArraySchema::new(
                    "Returns a list of systemd services.",
                    ObjectSchema::new("Service details.")
                        .required("service", StringSchema::new("Service ID."))
                        .required("name", StringSchema::new("systemd service name."))
                        .required("desc", StringSchema::new("systemd service description."))
                        .required("state", StringSchema::new("systemd service 'SubState'."))
                        .into()
                )
            )
        );

    route
}
