use failure::*;

use proxmox::{sortable, identity};

use crate::tools;
use crate::api_schema::*;
use crate::api_schema::router::*;
use serde_json::{json, Value};

use std::process::{Command, Stdio};

use crate::api2::types::*;

static SERVICE_NAME_LIST: [&str; 7] = [
    "proxmox-backup",
    "proxmox-backup-proxy",
    "sshd",
    "syslog",
    "cron",
    "postfix",
    "systemd-timesyncd",
];

fn real_service_name(service: &str) -> &str {

    // since postfix package 3.1.0-3.1 the postfix unit is only here
    // to manage subinstances, of which the default is called "-".
    // This is where we look for the daemon status

    if service == "postfix" {
        "postfix@-"
    } else {
        service
    }
}

fn get_full_service_state(service: &str) -> Result<Value, Error> {

    let real_service_name = real_service_name(service);

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

fn json_service_state(service: &str, status: Value) -> Value {

    if let Some(desc) = status["Description"].as_str() {
        let name = status["Name"].as_str().unwrap_or(service);
        let state = status["SubState"].as_str().unwrap_or("unknown");
        return json!({
            "service": service,
            "name": name,
            "desc": desc,
            "state": state,
        });
    }

    Value::Null
}


fn list_services(
    _param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let mut list = vec![];

    for service in &SERVICE_NAME_LIST {
        match get_full_service_state(service) {
            Ok(status) => {
                let state = json_service_state(service, status);
                if state != Value::Null {
                    list.push(state);
                }
            }
            Err(err) => log::error!("{}", err),
        }
    }

    Ok(Value::from(list))
}

fn get_service_state(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let service = tools::required_string_param(&param, "service")?;

    if !SERVICE_NAME_LIST.contains(&service) {
        bail!("unknown service name '{}'", service);
    }

    let status = get_full_service_state(service)?;

    Ok(json_service_state(service, status))
}

fn run_service_command(service: &str, cmd: &str) -> Result<Value, Error> {

    // fixme: run background worker (fork_worker) ???

    match cmd {
        "start"|"stop"|"restart"|"reload" => {},
        _ => bail!("unknown service command '{}'", cmd),
    }

    if service == "proxmox-backup" && cmd != "restart" {
	bail!("invalid service cmd '{} {}'", service, cmd);
    }

    let real_service_name = real_service_name(service);

    let status = Command::new("/bin/systemctl")
        .args(&[cmd, real_service_name])
        .status()?;

    if !status.success() {
        bail!("systemctl {} failed with {}", cmd, status);
    }

    Ok(Value::Null)
}

fn start_service(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let service = tools::required_string_param(&param, "service")?;

    log::info!("starting service {}", service);

    run_service_command(service, "start")
}

fn stop_service(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let service = tools::required_string_param(&param, "service")?;

    log::info!("stoping service {}", service);

    run_service_command(service, "stop")
}

fn restart_service(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let service = tools::required_string_param(&param, "service")?;

    log::info!("re-starting service {}", service);

    if service == "proxmox-backup-proxy" {
        // special case, avoid aborting running tasks
        run_service_command(service, "reload")
    } else {
        run_service_command(service, "restart")
    }
}

fn reload_service(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let service = tools::required_string_param(&param, "service")?;

    log::info!("reloading service {}", service);

    run_service_command(service, "reload")
}


const SERVICE_ID_SCHEMA: Schema = StringSchema::new("Service ID.")
    .max_length(256)
    .schema();

#[sortable]
const SERVICE_SUBDIRS: SubdirMap = &[
    (
        "reload", &Router::new()
            .post(
                &ApiMethod::new(
                    &ApiHandler::Sync(&reload_service),
                    &ObjectSchema::new(
                        "Reload service.",
                        &sorted!([
                            ("node", false, &NODE_SCHEMA),
                            ("service", false, &SERVICE_ID_SCHEMA),
                        ]),
                    )
                ).protected(true)
            )
    ),
    (
        "restart", &Router::new()
            .post(
                &ApiMethod::new(
                    &ApiHandler::Sync(&restart_service),
                    &ObjectSchema::new(
                        "Restart service.",
                        &sorted!([
                            ("node", false, &NODE_SCHEMA),
                            ("service", false, &SERVICE_ID_SCHEMA),
                        ]),
                    )
                ).protected(true)
            )
    ),
    (
        "start", &Router::new()
            .post(
                &ApiMethod::new(
                    &ApiHandler::Sync(&start_service),
                    &ObjectSchema::new(
                        "Start service.",
                        &sorted!([
                            ("node", false, &NODE_SCHEMA),
                            ("service", false, &SERVICE_ID_SCHEMA),
                        ]),
                    )
                ).protected(true)
            )
    ),
    (
        "state", &Router::new()
            .get(
                &ApiMethod::new(
                    &ApiHandler::Sync(&get_service_state),
                    &ObjectSchema::new(
                        "Read service properties.",
                        &sorted!([
                            ("node", false, &NODE_SCHEMA),
                            ("service", false, &SERVICE_ID_SCHEMA),
                        ]),
                    )
                )
            )
    ),
    (
        "stop", &Router::new()
            .post(
                &ApiMethod::new(
                    &ApiHandler::Sync(&stop_service),
                    &ObjectSchema::new(
                        "Stop service.",
                        &sorted!([
                            ("node", false, &NODE_SCHEMA),
                            ("service", false, &SERVICE_ID_SCHEMA),
                        ]),
                    )
                ).protected(true)
            )
    ),
];

const SERVICE_ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(SERVICE_SUBDIRS))
    .subdirs(SERVICE_SUBDIRS);

#[sortable]
pub const ROUTER: Router = Router::new()
    .get(
        &ApiMethod::new(
            &ApiHandler::Sync(&list_services),
            &ObjectSchema::new(
                "Service list.",
                &sorted!([ ("node", false, &NODE_SCHEMA) ]),
            )
        ).returns(
            &ArraySchema::new(
                "Returns a list of systemd services.",
                &ObjectSchema::new(
                    "Service details.",
                    &sorted!([
                        ("service", false, &SERVICE_ID_SCHEMA),
                        ("name", false, &StringSchema::new("systemd service name.").schema()),
                        ("desc", false, &StringSchema::new("systemd service description.").schema()),
                        ("state", false, &StringSchema::new("systemd service 'SubState'.").schema()),
                    ]),
                ).schema()
            ).schema()
        )
    )
    .match_all("service", &SERVICE_ROUTER);

