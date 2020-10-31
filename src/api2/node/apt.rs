use anyhow::{Error, bail, format_err};
use serde_json::{json, Value};

use proxmox::list_subdirs_api_method;
use proxmox::api::{api, RpcEnvironment, RpcEnvironmentType, Permission};
use proxmox::api::router::{Router, SubdirMap};

use crate::server::WorkerTask;
use crate::tools::{apt, http};

use crate::config::acl::{PRIV_SYS_AUDIT, PRIV_SYS_MODIFY};
use crate::api2::types::{Authid, APTUpdateInfo, NODE_SCHEMA, UPID_SCHEMA};

#[api(
    input: {
        properties: {
            node: {
                schema: NODE_SCHEMA,
            },
        },
    },
    returns: {
        description: "A list of packages with available updates.",
        type: Array,
        items: {
            type: APTUpdateInfo
        },
    },
    protected: true,
    access: {
        permission: &Permission::Privilege(&[], PRIV_SYS_AUDIT, false),
    },
)]
/// List available APT updates
fn apt_update_available(_param: Value) -> Result<Value, Error> {

    match apt::pkg_cache_expired() {
        Ok(false) => {
            if let Ok(Some(cache)) = apt::read_pkg_state() {
                return Ok(json!(cache.package_status));
            }
        },
        _ => (),
    }

    let cache = apt::update_cache()?;

    return Ok(json!(cache.package_status));
}

fn do_apt_update(worker: &WorkerTask, quiet: bool) -> Result<(), Error> {
    if !quiet { worker.log("starting apt-get update") }

    // TODO: set proxy /etc/apt/apt.conf.d/76pbsproxy like PVE

    let mut command = std::process::Command::new("apt-get");
    command.arg("update");

    // apt "errors" quite easily, and run_command is a bit rigid, so handle this inline for now.
    let output = command.output()
        .map_err(|err| format_err!("failed to execute {:?} - {}", command, err))?;

    if !quiet {
        worker.log(String::from_utf8(output.stdout)?);
    }

    // TODO: improve run_command to allow outputting both, stderr and stdout
    if !output.status.success() {
        if output.status.code().is_some() {
            let msg = String::from_utf8(output.stderr)
                .map(|m| if m.is_empty() { String::from("no error message") } else { m })
                .unwrap_or_else(|_| String::from("non utf8 error message (suppressed)"));
            worker.warn(msg);
        } else {
            bail!("terminated by signal");
        }
    }
    Ok(())
}

#[api(
    protected: true,
    input: {
        properties: {
            node: {
                schema: NODE_SCHEMA,
            },
            notify: {
                type: bool,
                description: r#"Send notification mail about new package updates availanle to the
                    email address configured for 'root@pam')."#,
                optional: true,
                default: false,
            },
            quiet: {
                description: "Only produces output suitable for logging, omitting progress indicators.",
                type: bool,
                default: false,
                optional: true,
            },
        },
    },
    returns: {
        schema: UPID_SCHEMA,
    },
    access: {
        permission: &Permission::Privilege(&[], PRIV_SYS_MODIFY, false),
    },
)]
/// Update the APT database
pub fn apt_update_database(
    notify: Option<bool>,
    quiet: Option<bool>,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<String, Error> {

    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let to_stdout = if rpcenv.env_type() == RpcEnvironmentType::CLI { true } else { false };
    // FIXME: change to non-option in signature and drop below once we have proxmox-api-macro 0.2.3
    let quiet = quiet.unwrap_or(API_METHOD_APT_UPDATE_DATABASE_PARAM_DEFAULT_QUIET);
    let notify = notify.unwrap_or(API_METHOD_APT_UPDATE_DATABASE_PARAM_DEFAULT_NOTIFY);

    let upid_str = WorkerTask::new_thread("aptupdate", None, auth_id, to_stdout, move |worker| {
        do_apt_update(&worker, quiet)?;

        let mut cache = apt::update_cache()?;

        if notify {
            let mut notified = match cache.notified {
                Some(notified) => notified,
                None => std::collections::HashMap::new(),
            };
            let mut to_notify: Vec<&APTUpdateInfo> = Vec::new();

            for pkg in &cache.package_status {
                match notified.insert(pkg.package.to_owned(), pkg.version.to_owned()) {
                    Some(notified_version) => {
                        if notified_version != pkg.version {
                            to_notify.push(pkg);
                        }
                    },
                    None => to_notify.push(pkg),
                }
            }
            if !to_notify.is_empty() {
                to_notify.sort_unstable_by_key(|k| &k.package);
                crate::server::send_updates_available(&to_notify)?;
            }
            cache.notified = Some(notified);
            apt::write_pkg_cache(&cache)?;
        }

        Ok(())
    })?;

    Ok(upid_str)
}

#[api(
    input: {
        properties: {
            node: {
                schema: NODE_SCHEMA,
            },
            name: {
                description: "Package name to get changelog of.",
                type: String,
            },
            version: {
                description: "Package version to get changelog of. Omit to use candidate version.",
                type: String,
                optional: true,
            },
        },
    },
    returns: {
        schema: UPID_SCHEMA,
    },
    access: {
        permission: &Permission::Privilege(&[], PRIV_SYS_MODIFY, false),
    },
)]
/// Retrieve the changelog of the specified package.
fn apt_get_changelog(
    param: Value,
) -> Result<Value, Error> {

    let name = crate::tools::required_string_param(&param, "name")?.to_owned();
    let version = param["version"].as_str();

    let pkg_info = apt::list_installed_apt_packages(|data| {
        match version {
            Some(version) => version == data.active_version,
            None => data.active_version == data.candidate_version
        }
    }, Some(&name));

    if pkg_info.len() == 0 {
        bail!("Package '{}' not found", name);
    }

    let changelog_url = &pkg_info[0].change_log_url;
    // FIXME: use 'apt-get changelog' for proxmox packages as well, once repo supports it
    if changelog_url.starts_with("http://download.proxmox.com/") {
        let changelog = crate::tools::runtime::block_on(http::get_string(changelog_url))
            .map_err(|err| format_err!("Error downloading changelog from '{}': {}", changelog_url, err))?;
        return Ok(json!(changelog));
    } else {
        let mut command = std::process::Command::new("apt-get");
        command.arg("changelog");
        command.arg("-qq"); // don't display download progress
        command.arg(name);
        let output = crate::tools::run_command(command, None)?;
        return Ok(json!(output));
    }
}

const SUBDIRS: SubdirMap = &[
    ("changelog", &Router::new().get(&API_METHOD_APT_GET_CHANGELOG)),
    ("update", &Router::new()
        .get(&API_METHOD_APT_UPDATE_AVAILABLE)
        .post(&API_METHOD_APT_UPDATE_DATABASE)
    ),
];

pub const ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(SUBDIRS))
    .subdirs(SUBDIRS);
