use std::collections::HashSet;

use apt_pkg_native::Cache;
use anyhow::{Error, bail, format_err};
use serde_json::{json, Value};

use proxmox::{list_subdirs_api_method, const_regex};
use proxmox::api::{api, RpcEnvironment, RpcEnvironmentType, Permission};
use proxmox::api::router::{Router, SubdirMap};

use crate::server::WorkerTask;
use crate::tools::http;

use crate::config::acl::{PRIV_SYS_AUDIT, PRIV_SYS_MODIFY};
use crate::api2::types::{APTUpdateInfo, NODE_SCHEMA, Userid, UPID_SCHEMA};

const_regex! {
    VERSION_EPOCH_REGEX = r"^\d+:";
    FILENAME_EXTRACT_REGEX = r"^.*/.*?_(.*)_Packages$";
}

// FIXME: once the 'changelog' API call switches over to 'apt-get changelog' only,
// consider removing this function entirely, as it's value is never used anywhere
// then (widget-toolkit doesn't use the value either)
fn get_changelog_url(
    package: &str,
    filename: &str,
    version: &str,
    origin: &str,
    component: &str,
) -> Result<String, Error> {
    if origin == "" {
        bail!("no origin available for package {}", package);
    }

    if origin == "Debian" {
        let mut command = std::process::Command::new("apt-get");
        command.arg("changelog");
        command.arg("--print-uris");
        command.arg(package);
        let output = crate::tools::run_command(command, None)?; // format: 'http://foo/bar' package.changelog
        let output = match output.splitn(2, ' ').next() {
            Some(output) => {
                if output.len() < 2 {
                    bail!("invalid output (URI part too short) from 'apt-get changelog --print-uris: {}", output)
                }
                output[1..output.len()-1].to_owned()
            },
            None => bail!("invalid output from 'apt-get changelog --print-uris': {}", output)
        };
        return Ok(output);
    } else if origin == "Proxmox" {
        // FIXME: Use above call to 'apt changelog <pkg> --print-uris' as well.
        // Currently not possible as our packages do not have a URI set in their Release file.
        let version = (VERSION_EPOCH_REGEX.regex_obj)().replace_all(version, "");

        let base = match (FILENAME_EXTRACT_REGEX.regex_obj)().captures(filename) {
            Some(captures) => {
                let base_capture = captures.get(1);
                match base_capture {
                    Some(base_underscore) => base_underscore.as_str().replace("_", "/"),
                    None => bail!("incompatible filename, cannot find regex group")
                }
            },
            None => bail!("incompatible filename, doesn't match regex")
        };

        return Ok(format!("http://download.proxmox.com/{}/{}_{}.changelog",
                          base, package, version));
    }

    bail!("unknown origin ({}) or component ({})", origin, component)
}

struct FilterData<'a> {
    // this is version info returned by APT
    installed_version: Option<&'a str>,
    candidate_version: &'a str,

    // this is the version info the filter is supposed to check
    active_version: &'a str,
}

enum PackagePreSelect {
    OnlyInstalled,
    OnlyNew,
    All,
}

fn list_installed_apt_packages<F: Fn(FilterData) -> bool>(
    filter: F,
    only_versions_for: Option<&str>,
) -> Vec<APTUpdateInfo> {

    let mut ret = Vec::new();
    let mut depends = HashSet::new();

    // note: this is not an 'apt update', it just re-reads the cache from disk
    let mut cache = Cache::get_singleton();
    cache.reload();

    let mut cache_iter = match only_versions_for {
        Some(name) => cache.find_by_name(name),
        None => cache.iter()
    };

    loop {

        match cache_iter.next() {
            Some(view) => {
                let di = if only_versions_for.is_some() {
                    query_detailed_info(
                        PackagePreSelect::All,
                        &filter,
                        view,
                        None
                    )
                } else {
                    query_detailed_info(
                        PackagePreSelect::OnlyInstalled,
                        &filter,
                        view,
                        Some(&mut depends)
                    )
                };
                if let Some(info) = di {
                    ret.push(info);
                }

                if only_versions_for.is_some() {
                    break;
                }
            },
            None => {
                drop(cache_iter);
                // also loop through missing dependencies, as they would be installed
                for pkg in depends.iter() {
                    let mut iter = cache.find_by_name(&pkg);
                    let view = match iter.next() {
                        Some(view) => view,
                        None => continue // package not found, ignore
                    };

                    let di = query_detailed_info(
                        PackagePreSelect::OnlyNew,
                        &filter,
                        view,
                        None
                    );
                    if let Some(info) = di {
                        ret.push(info);
                    }
                }
                break;
            }
        }
    }

    return ret;
}

fn query_detailed_info<'a, F, V>(
    pre_select: PackagePreSelect,
    filter: F,
    view: V,
    depends: Option<&mut HashSet<String>>,
) -> Option<APTUpdateInfo>
where
    F: Fn(FilterData) -> bool,
    V: std::ops::Deref<Target = apt_pkg_native::sane::PkgView<'a>>
{
    let current_version = view.current_version();
    let candidate_version = view.candidate_version();

    let (current_version, candidate_version) = match pre_select {
        PackagePreSelect::OnlyInstalled => match (current_version, candidate_version) {
            (Some(cur), Some(can)) => (Some(cur), can), // package installed and there is an update
            (Some(cur), None) => (Some(cur.clone()), cur), // package installed and up-to-date
            (None, Some(_)) => return None, // package could be installed
            (None, None) => return None, // broken
        },
        PackagePreSelect::OnlyNew => match (current_version, candidate_version) {
            (Some(_), Some(_)) => return None,
            (Some(_), None) => return None,
            (None, Some(can)) => (None, can),
            (None, None) => return None,
        },
        PackagePreSelect::All => match (current_version, candidate_version) {
            (Some(cur), Some(can)) => (Some(cur), can),
            (Some(cur), None) => (Some(cur.clone()), cur),
            (None, Some(can)) => (None, can),
            (None, None) => return None,
        },
    };

    // get additional information via nested APT 'iterators'
    let mut view_iter = view.versions();
    while let Some(ver) = view_iter.next() {

        let package = view.name();
        let version = ver.version();
        let mut origin_res = "unknown".to_owned();
        let mut section_res = "unknown".to_owned();
        let mut priority_res = "unknown".to_owned();
        let mut change_log_url = "".to_owned();
        let mut short_desc = package.clone();
        let mut long_desc = "".to_owned();

        let fd = FilterData {
            installed_version: current_version.as_deref(),
            candidate_version: &candidate_version,
            active_version: &version,
        };

        if filter(fd) {
            if let Some(section) = ver.section() {
                section_res = section;
            }

            if let Some(prio) = ver.priority_type() {
                priority_res = prio;
            }

            // assume every package has only one origin file (not
            // origin, but origin *file*, for some reason those seem to
            // be different concepts in APT)
            let mut origin_iter = ver.origin_iter();
            let origin = origin_iter.next();
            if let Some(origin) = origin {

                if let Some(sd) = origin.short_desc() {
                    short_desc = sd;
                }

                if let Some(ld) = origin.long_desc() {
                    long_desc = ld;
                }

                // the package files appear in priority order, meaning
                // the one for the candidate version is first - this is fine
                // however, as the source package should be the same for all
                // versions anyway
                let mut pkg_iter = origin.file();
                let pkg_file = pkg_iter.next();
                if let Some(pkg_file) = pkg_file {
                    if let Some(origin_name) = pkg_file.origin() {
                        origin_res = origin_name;
                    }

                    let filename = pkg_file.file_name();
                    let component = pkg_file.component();

                    // build changelog URL from gathered information
                    // ignore errors, use empty changelog instead
                    let url = get_changelog_url(&package, &filename,
                        &version, &origin_res, &component);
                    if let Ok(url) = url {
                        change_log_url = url;
                    }
                }
            }

            if let Some(depends) = depends {
                let mut dep_iter = ver.dep_iter();
                loop {
                    let dep = match dep_iter.next() {
                        Some(dep) if dep.dep_type() != "Depends" => continue,
                        Some(dep) => dep,
                        None => break
                    };

                    let dep_pkg = dep.target_pkg();
                    let name = dep_pkg.name();

                    depends.insert(name);
                }
            }

            return Some(APTUpdateInfo {
                package,
                title: short_desc,
                arch: view.arch(),
                description: long_desc,
                change_log_url,
                origin: origin_res,
                version: candidate_version.clone(),
                old_version: match current_version {
                    Some(vers) => vers,
                    None => "".to_owned()
                },
                priority: priority_res,
                section: section_res,
            });
        }
    }

    return None;
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
        description: "A list of packages with available updates.",
        type: Array,
        items: { type: APTUpdateInfo },
    },
    access: {
        permission: &Permission::Privilege(&[], PRIV_SYS_AUDIT, false),
    },
)]
/// List available APT updates
fn apt_update_available(_param: Value) -> Result<Value, Error> {
    let all_upgradeable = list_installed_apt_packages(|data| {
        data.candidate_version == data.active_version &&
        data.installed_version != Some(data.candidate_version)
    }, None);
    Ok(json!(all_upgradeable))
}

#[api(
    protected: true,
    input: {
        properties: {
            node: {
                schema: NODE_SCHEMA,
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
    quiet: Option<bool>,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<String, Error> {

    let userid: Userid = rpcenv.get_user().unwrap().parse()?;
    let to_stdout = if rpcenv.env_type() == RpcEnvironmentType::CLI { true } else { false };
    let quiet = quiet.unwrap_or(API_METHOD_APT_UPDATE_DATABASE_PARAM_DEFAULT_QUIET);

    let upid_str = WorkerTask::new_thread("aptupdate", None, userid, to_stdout, move |worker| {
        if !quiet { worker.log("starting apt-get update") }

        // TODO: set proxy /etc/apt/apt.conf.d/76pbsproxy like PVE

        let mut command = std::process::Command::new("apt-get");
        command.arg("update");

        let output = crate::tools::run_command(command, None)?;
        if !quiet { worker.log(output) }

        // TODO: add mail notify for new updates like PVE

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

    let pkg_info = list_installed_apt_packages(|data| {
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
            .map_err(|err| format_err!("Error downloading changelog: {}", err))?;
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
