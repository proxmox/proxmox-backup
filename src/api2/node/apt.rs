use anyhow::{Error, bail, format_err};
use serde_json::{json, Value};
use std::collections::HashMap;

use proxmox::list_subdirs_api_method;
use proxmox::api::{api, RpcEnvironment, RpcEnvironmentType, Permission};
use proxmox::api::router::{Router, SubdirMap};

use crate::server::WorkerTask;
use crate::tools::{apt, http, subscription};

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

    if let Ok(false) = apt::pkg_cache_expired() {
        if let Ok(Some(cache)) = apt::read_pkg_state() {
            return Ok(json!(cache.package_status));
        }
    }

    let cache = apt::update_cache()?;

    Ok(json!(cache.package_status))
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
                description: r#"Send notification mail about new package updates available to the
                    email address configured for 'root@pam')."#,
                default: false,
                optional: true,
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
    notify: bool,
    quiet: bool,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<String, Error> {

    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let to_stdout = rpcenv.env_type() == RpcEnvironmentType::CLI;

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

    if pkg_info.is_empty() {
        bail!("Package '{}' not found", name);
    }

    let changelog_url = &pkg_info[0].change_log_url;
    // FIXME: use 'apt-get changelog' for proxmox packages as well, once repo supports it
    if changelog_url.starts_with("http://download.proxmox.com/") {
        let changelog = crate::tools::runtime::block_on(http::get_string(changelog_url, None))
            .map_err(|err| format_err!("Error downloading changelog from '{}': {}", changelog_url, err))?;
        Ok(json!(changelog))

    } else if changelog_url.starts_with("https://enterprise.proxmox.com/") {
        let sub = match subscription::read_subscription()? {
            Some(sub) => sub,
            None => bail!("cannot retrieve changelog from enterprise repo: no subscription info found")
        };
        let (key, id) = match sub.key {
            Some(key) => {
                match sub.serverid {
                    Some(id) => (key, id),
                    None =>
                        bail!("cannot retrieve changelog from enterprise repo: no server id found")
                }
            },
            None => bail!("cannot retrieve changelog from enterprise repo: no subscription key found")
        };

        let mut auth_header = HashMap::new();
        auth_header.insert("Authorization".to_owned(),
            format!("Basic {}", base64::encode(format!("{}:{}", key, id))));

        let changelog = crate::tools::runtime::block_on(http::get_string(changelog_url, Some(&auth_header)))
            .map_err(|err| format_err!("Error downloading changelog from '{}': {}", changelog_url, err))?;
        Ok(json!(changelog))

    } else {
        let mut command = std::process::Command::new("apt-get");
        command.arg("changelog");
        command.arg("-qq"); // don't display download progress
        command.arg(name);
        let output = crate::tools::run_command(command, None)?;
        Ok(json!(output))
    }
}

#[api(
    input: {
        properties: {
            node: {
                schema: NODE_SCHEMA,
            },
        },
    },
    returns: {
        description: "List of more relevant packages.",
        type: Array,
        items: {
            type: APTUpdateInfo,
        },
    },
    access: {
        permission: &Permission::Privilege(&[], PRIV_SYS_AUDIT, false),
    },
)]
/// Get package information for important Proxmox Backup Server packages.
pub fn get_versions() -> Result<Vec<APTUpdateInfo>, Error> {
    const PACKAGES: &[&str] = &[
        "ifupdown2",
        "libjs-extjs",
        "proxmox-backup",
        "proxmox-backup-docs",
        "proxmox-backup-client",
        "proxmox-backup-server",
        "proxmox-mini-journalreader",
        "proxmox-widget-toolkit",
        "pve-xtermjs",
        "smartmontools",
        "zfsutils-linux",
    ];

    fn unknown_package(package: String, extra_info: Option<String>) -> APTUpdateInfo {
        APTUpdateInfo {
            package,
            title: "unknown".into(),
            arch: "unknown".into(),
            description: "unknown".into(),
            version: "unknown".into(),
            old_version: "unknown".into(),
            origin: "unknown".into(),
            priority: "unknown".into(),
            section: "unknown".into(),
            change_log_url: "unknown".into(),
            extra_info,
        }
    }

    let is_kernel = |name: &str| name.starts_with("pve-kernel-");

    let mut packages: Vec<APTUpdateInfo> = Vec::new();
    let pbs_packages = apt::list_installed_apt_packages(
        |filter| {
            filter.installed_version == Some(filter.active_version)
                && (is_kernel(filter.package) || PACKAGES.contains(&filter.package))
        },
        None,
    );

    let running_kernel = format!(
        "running kernel: {}",
        nix::sys::utsname::uname().release().to_owned()
    );
    if let Some(proxmox_backup) = pbs_packages.iter().find(|pkg| pkg.package == "proxmox-backup") {
        let mut proxmox_backup = proxmox_backup.clone();
        proxmox_backup.extra_info = Some(running_kernel);
        packages.push(proxmox_backup);
    } else {
        packages.push(unknown_package("proxmox-backup".into(), Some(running_kernel)));
    }

    let version = crate::api2::version::PROXMOX_PKG_VERSION;
    let release = crate::api2::version::PROXMOX_PKG_RELEASE;
    let daemon_version_info = Some(format!("running version: {}.{}", version, release));
    if let Some(pkg) = pbs_packages.iter().find(|pkg| pkg.package == "proxmox-backup-server") {
        let mut pkg = pkg.clone();
        pkg.extra_info = daemon_version_info;
        packages.push(pkg);
    } else {
        packages.push(unknown_package("proxmox-backup".into(), daemon_version_info));
    }

    let mut kernel_pkgs: Vec<APTUpdateInfo> = pbs_packages
        .iter()
        .filter(|pkg| is_kernel(&pkg.package))
        .cloned()
        .collect();
    // make sure the cache mutex gets dropped before the next call to list_installed_apt_packages
    {
        let cache = apt_pkg_native::Cache::get_singleton();
        kernel_pkgs.sort_by(|left, right| {
            cache
                .compare_versions(&left.old_version, &right.old_version)
                .reverse()
        });
    }
    packages.append(&mut kernel_pkgs);

    // add entry for all packages we're interested in, even if not installed
    for pkg in PACKAGES.iter() {
        if pkg == &"proxmox-backup" || pkg == &"proxmox-backup-server" {
            continue;
        }
        match pbs_packages.iter().find(|item| &item.package == pkg) {
            Some(apt_pkg) => packages.push(apt_pkg.to_owned()),
            None => packages.push(unknown_package(pkg.to_string(), None)),
        }
    }

    Ok(packages)
}

const SUBDIRS: SubdirMap = &[
    ("changelog", &Router::new().get(&API_METHOD_APT_GET_CHANGELOG)),
    ("update", &Router::new()
        .get(&API_METHOD_APT_UPDATE_AVAILABLE)
        .post(&API_METHOD_APT_UPDATE_DATABASE)
    ),
    ("versions", &Router::new().get(&API_METHOD_GET_VERSIONS)),
];

pub const ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(SUBDIRS))
    .subdirs(SUBDIRS);
