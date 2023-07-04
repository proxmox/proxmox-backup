use std::collections::HashMap;
use std::collections::HashSet;

use anyhow::{bail, format_err, Error};
use apt_pkg_native::Cache;

use proxmox_schema::const_regex;
use proxmox_sys::fs::{file_read_optional_string, replace_file, CreateOptions};

use pbs_api_types::APTUpdateInfo;
use pbs_buildcfg::PROXMOX_BACKUP_STATE_DIR_M;

const APT_PKG_STATE_FN: &str = concat!(PROXMOX_BACKUP_STATE_DIR_M!(), "/pkg-state.json");

#[derive(Debug, serde::Serialize, serde::Deserialize)]
/// Some information we cache about the package (update) state, like what pending update version
/// we already notfied an user about
pub struct PkgState {
    /// simple map from package name to most recently notified (emailed) version
    pub notified: Option<HashMap<String, String>>,
    /// A list of pending updates
    pub package_status: Vec<APTUpdateInfo>,
}

pub fn write_pkg_cache(state: &PkgState) -> Result<(), Error> {
    let serialized_state = serde_json::to_string(state)?;

    replace_file(
        APT_PKG_STATE_FN,
        serialized_state.as_bytes(),
        CreateOptions::new(),
        false,
    )
    .map_err(|err| format_err!("Error writing package cache - {}", err))?;
    Ok(())
}

pub fn read_pkg_state() -> Result<Option<PkgState>, Error> {
    let serialized_state = match file_read_optional_string(APT_PKG_STATE_FN) {
        Ok(Some(raw)) => raw,
        Ok(None) => return Ok(None),
        Err(err) => bail!("could not read cached package state file - {}", err),
    };

    serde_json::from_str(&serialized_state)
        .map(Some)
        .map_err(|err| format_err!("could not parse cached package status - {}", err))
}

pub fn pkg_cache_expired() -> Result<bool, Error> {
    if let Ok(pbs_cache) = std::fs::metadata(APT_PKG_STATE_FN) {
        let apt_pkgcache = std::fs::metadata("/var/cache/apt/pkgcache.bin")?;
        let dpkg_status = std::fs::metadata("/var/lib/dpkg/status")?;

        let mtime = pbs_cache.modified()?;

        if apt_pkgcache.modified()? <= mtime && dpkg_status.modified()? <= mtime {
            return Ok(false);
        }
    }
    Ok(true)
}

pub fn update_cache() -> Result<PkgState, Error> {
    // update our cache
    let all_upgradeable = list_installed_apt_packages(
        |data| {
            data.candidate_version == data.active_version
                && data.installed_version != Some(data.candidate_version)
        },
        None,
    );

    let cache = match read_pkg_state() {
        Ok(Some(mut cache)) => {
            cache.package_status = all_upgradeable;
            cache
        }
        _ => PkgState {
            notified: None,
            package_status: all_upgradeable,
        },
    };
    write_pkg_cache(&cache)?;
    Ok(cache)
}

const_regex! {
    VERSION_EPOCH_REGEX = r"^\d+:";
    FILENAME_EXTRACT_REGEX = r"^.*/.*?_(.*)_Packages$";
}

pub struct FilterData<'a> {
    /// package name
    pub package: &'a str,
    /// this is version info returned by APT
    pub installed_version: Option<&'a str>,
    pub candidate_version: &'a str,

    /// this is the version info the filter is supposed to check
    pub active_version: &'a str,
}

enum PackagePreSelect {
    OnlyInstalled,
    OnlyNew,
    All,
}

pub fn list_installed_apt_packages<F: Fn(FilterData) -> bool>(
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
        None => cache.iter(),
    };

    loop {
        match cache_iter.next() {
            Some(view) => {
                let di = if only_versions_for.is_some() {
                    query_detailed_info(PackagePreSelect::All, &filter, view, None)
                } else {
                    query_detailed_info(
                        PackagePreSelect::OnlyInstalled,
                        &filter,
                        view,
                        Some(&mut depends),
                    )
                };
                if let Some(info) = di {
                    ret.push(info);
                }

                if only_versions_for.is_some() {
                    break;
                }
            }
            None => {
                drop(cache_iter);
                // also loop through missing dependencies, as they would be installed
                for pkg in depends.iter() {
                    let mut iter = cache.find_by_name(pkg);
                    let view = match iter.next() {
                        Some(view) => view,
                        None => continue, // package not found, ignore
                    };

                    let di = query_detailed_info(PackagePreSelect::OnlyNew, &filter, view, None);
                    if let Some(info) = di {
                        ret.push(info);
                    }
                }
                break;
            }
        }
    }

    ret
}

fn query_detailed_info<'a, F, V>(
    pre_select: PackagePreSelect,
    filter: F,
    view: V,
    depends: Option<&mut HashSet<String>>,
) -> Option<APTUpdateInfo>
where
    F: Fn(FilterData) -> bool,
    V: std::ops::Deref<Target = apt_pkg_native::sane::PkgView<'a>>,
{
    let current_version = view.current_version();
    let candidate_version = view.candidate_version();

    let (current_version, candidate_version) = match pre_select {
        PackagePreSelect::OnlyInstalled => match (current_version, candidate_version) {
            (Some(cur), Some(can)) => (Some(cur), can), // package installed and there is an update
            (Some(cur), None) => (Some(cur.clone()), cur), // package installed and up-to-date
            (None, Some(_)) => return None,             // package could be installed
            (None, None) => return None,                // broken
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
        let mut short_desc = package.clone();
        let mut long_desc = "".to_owned();

        let fd = FilterData {
            package: package.as_str(),
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
                }
            }

            if let Some(depends) = depends {
                let mut dep_iter = ver.dep_iter();
                loop {
                    let dep = match dep_iter.next() {
                        Some(dep) if dep.dep_type() != "Depends" => continue,
                        Some(dep) => dep,
                        None => break,
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
                origin: origin_res,
                version: candidate_version.clone(),
                old_version: match current_version {
                    Some(vers) => vers,
                    None => "".to_owned(),
                },
                priority: priority_res,
                section: section_res,
                extra_info: None,
            });
        }
    }

    None
}
