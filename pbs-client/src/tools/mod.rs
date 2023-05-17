//! Shared tools useful for common CLI clients.
use std::collections::HashMap;
use std::env::VarError::{NotPresent, NotUnicode};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::os::unix::io::FromRawFd;
use std::process::Command;

use anyhow::{bail, format_err, Context, Error};
use serde_json::{json, Value};
use xdg::BaseDirectories;

use proxmox_http::uri::json_object_to_query;
use proxmox_router::cli::{complete_file_name, shellword_split};
use proxmox_schema::*;
use proxmox_sys::fs::file_get_json;

use pbs_api_types::{Authid, BackupNamespace, RateLimitConfig, UserWithTokens, BACKUP_REPO_URL};

use crate::{BackupRepository, HttpClient, HttpClientOptions};

pub mod key_source;

const ENV_VAR_PBS_FINGERPRINT: &str = "PBS_FINGERPRINT";
const ENV_VAR_PBS_PASSWORD: &str = "PBS_PASSWORD";

pub const REPO_URL_SCHEMA: Schema = StringSchema::new("Repository URL.")
    .format(&BACKUP_REPO_URL)
    .max_length(256)
    .schema();

pub const CHUNK_SIZE_SCHEMA: Schema = IntegerSchema::new("Chunk size in KB. Must be a power of 2.")
    .minimum(64)
    .maximum(4096)
    .default(4096)
    .schema();

/// Helper to read a secret through a environment variable (ENV).
///
/// Tries the following variable names in order and returns the value
/// it will resolve for the first defined one:
///
/// BASE_NAME => use value from ENV(BASE_NAME) directly as secret
/// BASE_NAME_FD => read the secret from the specified file descriptor
/// BASE_NAME_FILE => read the secret from the specified file name
/// BASE_NAME_CMD => read the secret from specified command first line of output on stdout
///
/// Only return the first line of data (without CRLF).
pub fn get_secret_from_env(base_name: &str) -> Result<Option<String>, Error> {
    let firstline = |data: String| -> String {
        match data.lines().next() {
            Some(line) => line.to_string(),
            None => String::new(),
        }
    };

    let firstline_file = |file: &mut File| -> Result<String, Error> {
        let reader = BufReader::new(file);
        match reader.lines().next() {
            Some(Ok(line)) => Ok(line),
            Some(Err(err)) => Err(err.into()),
            None => Ok(String::new()),
        }
    };

    match std::env::var(base_name) {
        Ok(p) => return Ok(Some(firstline(p))),
        Err(NotUnicode(_)) => bail!(format!("{} contains bad characters", base_name)),
        Err(NotPresent) => {}
    };

    let env_name = format!("{}_FD", base_name);
    match std::env::var(&env_name) {
        Ok(fd_str) => {
            let fd: i32 = fd_str.parse().map_err(|err| {
                format_err!(
                    "unable to parse file descriptor in ENV({}): {}",
                    env_name,
                    err
                )
            })?;
            let mut file = unsafe { File::from_raw_fd(fd) };
            return Ok(Some(firstline_file(&mut file)?));
        }
        Err(NotUnicode(_)) => bail!(format!("{} contains bad characters", env_name)),
        Err(NotPresent) => {}
    }

    let env_name = format!("{}_FILE", base_name);
    match std::env::var(&env_name) {
        Ok(filename) => {
            let mut file = std::fs::File::open(filename)
                .map_err(|err| format_err!("unable to open file in ENV({}): {}", env_name, err))?;
            return Ok(Some(firstline_file(&mut file)?));
        }
        Err(NotUnicode(_)) => bail!(format!("{} contains bad characters", env_name)),
        Err(NotPresent) => {}
    }

    let env_name = format!("{}_CMD", base_name);
    match std::env::var(&env_name) {
        Ok(ref command) => {
            let args = shellword_split(command)?;
            let mut command = Command::new(&args[0]);
            command.args(&args[1..]);
            let output = proxmox_sys::command::run_command(command, None)?;
            return Ok(Some(firstline(output)));
        }
        Err(NotUnicode(_)) => bail!(format!("{} contains bad characters", env_name)),
        Err(NotPresent) => {}
    }

    Ok(None)
}

pub fn get_default_repository() -> Option<String> {
    std::env::var("PBS_REPOSITORY").ok()
}

pub fn extract_repository_from_value(param: &Value) -> Result<BackupRepository, Error> {
    let repo_url = param["repository"]
        .as_str()
        .map(String::from)
        .or_else(get_default_repository)
        .ok_or_else(|| format_err!("unable to get (default) repository"))?;

    let repo: BackupRepository = repo_url.parse()?;

    Ok(repo)
}

pub fn extract_repository_from_map(param: &HashMap<String, String>) -> Option<BackupRepository> {
    param
        .get("repository")
        .map(String::from)
        .or_else(get_default_repository)
        .and_then(|repo_url| repo_url.parse::<BackupRepository>().ok())
}

pub fn connect(repo: &BackupRepository) -> Result<HttpClient, Error> {
    let rate_limit = RateLimitConfig::default(); // unlimited
    connect_do(repo.host(), repo.port(), repo.auth_id(), rate_limit)
        .map_err(|err| format_err!("error building client for repository {} - {}", repo, err))
}

pub fn connect_rate_limited(
    repo: &BackupRepository,
    rate_limit: RateLimitConfig,
) -> Result<HttpClient, Error> {
    connect_do(repo.host(), repo.port(), repo.auth_id(), rate_limit)
        .map_err(|err| format_err!("error building client for repository {} - {}", repo, err))
}

fn connect_do(
    server: &str,
    port: u16,
    auth_id: &Authid,
    rate_limit: RateLimitConfig,
) -> Result<HttpClient, Error> {
    let fingerprint = std::env::var(ENV_VAR_PBS_FINGERPRINT).ok();

    let password = get_secret_from_env(ENV_VAR_PBS_PASSWORD)?;
    let options = HttpClientOptions::new_interactive(password, fingerprint).rate_limit(rate_limit);

    HttpClient::new(server, port, auth_id, options)
}

/// like get, but simply ignore errors and return Null instead
pub async fn try_get(repo: &BackupRepository, url: &str) -> Value {
    let fingerprint = std::env::var(ENV_VAR_PBS_FINGERPRINT).ok();
    let password = get_secret_from_env(ENV_VAR_PBS_PASSWORD).unwrap_or(None);

    // ticket cache, but no questions asked
    let options = HttpClientOptions::new_interactive(password, fingerprint).interactive(false);

    let client = match HttpClient::new(repo.host(), repo.port(), repo.auth_id(), options) {
        Ok(v) => v,
        _ => return Value::Null,
    };

    let mut resp = match client.get(url, None).await {
        Ok(v) => v,
        _ => return Value::Null,
    };

    if let Some(map) = resp.as_object_mut() {
        if let Some(data) = map.remove("data") {
            return data;
        }
    }
    Value::Null
}

pub fn complete_backup_group(_arg: &str, param: &HashMap<String, String>) -> Vec<String> {
    proxmox_async::runtime::main(async { complete_backup_group_do(param).await })
}

pub async fn complete_backup_group_do(param: &HashMap<String, String>) -> Vec<String> {
    let mut result = vec![];

    let repo = match extract_repository_from_map(param) {
        Some(v) => v,
        _ => return result,
    };

    let path = format!("api2/json/admin/datastore/{}/groups", repo.store());

    let data = try_get(&repo, &path).await;

    if let Some(list) = data.as_array() {
        for item in list {
            if let (Some(backup_id), Some(backup_type)) =
                (item["backup-id"].as_str(), item["backup-type"].as_str())
            {
                result.push(format!("{}/{}", backup_type, backup_id));
            }
        }
    }

    result
}

pub fn complete_group_or_snapshot(arg: &str, param: &HashMap<String, String>) -> Vec<String> {
    proxmox_async::runtime::main(async { complete_group_or_snapshot_do(arg, param).await })
}

pub async fn complete_group_or_snapshot_do(
    arg: &str,
    param: &HashMap<String, String>,
) -> Vec<String> {
    if arg.matches('/').count() < 2 {
        let groups = complete_backup_group_do(param).await;
        let mut result = vec![];
        for group in groups {
            result.push(group.to_string());
            result.push(format!("{}/", group));
        }
        return result;
    }

    complete_backup_snapshot_do(param).await
}

pub fn complete_backup_snapshot(_arg: &str, param: &HashMap<String, String>) -> Vec<String> {
    proxmox_async::runtime::main(async { complete_backup_snapshot_do(param).await })
}

pub async fn complete_backup_snapshot_do(param: &HashMap<String, String>) -> Vec<String> {
    let mut result = vec![];

    let repo = match extract_repository_from_map(param) {
        Some(v) => v,
        _ => return result,
    };

    let path = format!("api2/json/admin/datastore/{}/snapshots", repo.store());

    let data = try_get(&repo, &path).await;

    if let Value::Array(list) = data {
        for item in list {
            match serde_json::from_value::<pbs_api_types::BackupDir>(item) {
                Ok(item) => result.push(item.to_string()),
                Err(_) => {
                    // FIXME: print error in completion?
                    continue;
                }
            };
        }
    }

    result
}

pub fn complete_server_file_name(_arg: &str, param: &HashMap<String, String>) -> Vec<String> {
    proxmox_async::runtime::main(async { complete_server_file_name_do(param).await })
}

pub async fn complete_server_file_name_do(param: &HashMap<String, String>) -> Vec<String> {
    let mut result = vec![];

    let repo = match extract_repository_from_map(param) {
        Some(v) => v,
        _ => return result,
    };

    let snapshot: pbs_api_types::BackupDir = match param.get("snapshot") {
        Some(path) => match path.parse() {
            Ok(v) => v,
            _ => return result,
        },
        _ => return result,
    };

    let ns: pbs_api_types::BackupNamespace = match param.get("ns") {
        Some(ns) => match ns.parse() {
            Ok(v) => v,
            _ => return result,
        },
        _ => {
            // If no namespace flag is provided, we assume the root namespace
            pbs_api_types::BackupNamespace::root()
        }
    };

    let query = json_object_to_query(json!({
        "ns": ns,
        "backup-type": snapshot.group.ty,
        "backup-id": snapshot.group.id,
        "backup-time": snapshot.time,
    }))
    .unwrap();

    let path = format!("api2/json/admin/datastore/{}/files?{}", repo.store(), query);

    let data = try_get(&repo, &path).await;

    if let Some(list) = data.as_array() {
        for item in list {
            if let Some(filename) = item["filename"].as_str() {
                result.push(filename.to_owned());
            }
        }
    }

    result
}

pub fn complete_archive_name(arg: &str, param: &HashMap<String, String>) -> Vec<String> {
    complete_server_file_name(arg, param)
        .iter()
        .map(|v| pbs_tools::format::strip_server_file_extension(v).to_owned())
        .collect()
}

pub fn complete_pxar_archive_name(arg: &str, param: &HashMap<String, String>) -> Vec<String> {
    complete_server_file_name(arg, param)
        .iter()
        .filter_map(|name| {
            if name.ends_with(".pxar.didx") {
                Some(pbs_tools::format::strip_server_file_extension(name).to_owned())
            } else {
                None
            }
        })
        .collect()
}

pub fn complete_img_archive_name(arg: &str, param: &HashMap<String, String>) -> Vec<String> {
    complete_server_file_name(arg, param)
        .iter()
        .filter_map(|name| {
            if name.ends_with(".img.fidx") {
                Some(pbs_tools::format::strip_server_file_extension(name).to_owned())
            } else {
                None
            }
        })
        .collect()
}

pub fn complete_chunk_size(_arg: &str, _param: &HashMap<String, String>) -> Vec<String> {
    let mut result = vec![];

    let mut size = 64;
    loop {
        result.push(size.to_string());
        size *= 2;
        if size > 4096 {
            break;
        }
    }

    result
}

pub fn complete_auth_id(_arg: &str, param: &HashMap<String, String>) -> Vec<String> {
    proxmox_async::runtime::main(async { complete_auth_id_do(param).await })
}

pub async fn complete_auth_id_do(param: &HashMap<String, String>) -> Vec<String> {
    let mut result = vec![];

    let repo = match extract_repository_from_map(param) {
        Some(v) => v,
        _ => return result,
    };

    let data = try_get(&repo, "api2/json/access/users?include_tokens=true").await;

    if let Ok(parsed) = serde_json::from_value::<Vec<UserWithTokens>>(data) {
        for user in parsed {
            result.push(user.userid.to_string());
            for token in user.tokens {
                result.push(token.tokenid.to_string());
            }
        }
    };

    result
}

pub fn complete_repository(_arg: &str, _param: &HashMap<String, String>) -> Vec<String> {
    let mut result = vec![];

    let base = match BaseDirectories::with_prefix("proxmox-backup") {
        Ok(v) => v,
        _ => return result,
    };

    // usually $HOME/.cache/proxmox-backup/repo-list
    let path = match base.place_cache_file("repo-list") {
        Ok(v) => v,
        _ => return result,
    };

    let data = file_get_json(path, None).unwrap_or_else(|_| json!({}));

    if let Some(map) = data.as_object() {
        for (repo, _count) in map {
            result.push(repo.to_owned());
        }
    }

    result
}

pub fn complete_backup_source(arg: &str, param: &HashMap<String, String>) -> Vec<String> {
    let mut result = vec![];

    let data: Vec<&str> = arg.splitn(2, ':').collect();

    if data.len() != 2 {
        result.push(String::from("root.pxar:/"));
        result.push(String::from("etc.pxar:/etc"));
        return result;
    }

    let files = complete_file_name(data[1], param);

    for file in files {
        result.push(format!("{}:{}", data[0], file));
    }

    result
}

pub fn complete_namespace(arg: &str, param: &HashMap<String, String>) -> Vec<String> {
    // the prefix includes the parent since we get the full namespace as API results
    let prefix = arg;
    let parent = match arg.rfind('/') {
        // we're at a slash, so use the full namespace as a parent, no filter
        Some(len) if len == arg.len() => &arg[..(len - 1)],
        // there was a slash in the namespace, pop off the final component, use the
        // remainder as a filter:
        Some(len) => &arg[..len],
        // no slashes, search root namespace
        None => "",
    };

    let parent: BackupNamespace = match parent.parse() {
        Ok(ns) => ns,
        Err(_) => return Vec::new(),
    };

    proxmox_async::runtime::main(complete_namespace_do(parent, prefix, param))
}

pub async fn complete_namespace_do(
    parent: BackupNamespace,
    prefix: &str,
    param: &HashMap<String, String>,
) -> Vec<String> {
    let repo = match extract_repository_from_map(param) {
        Some(v) => v,
        _ => return Vec::new(),
    };

    let mut param = json!({ "max-depth": 2 });
    if !parent.is_root() {
        param["parent"] = match serde_json::to_value(parent) {
            Ok(p) => p,
            Err(_) => return Vec::new(),
        };
    }
    let query = json_object_to_query(param).unwrap();
    let path = format!(
        "api2/json/admin/datastore/{}/namespace?{query}",
        repo.store()
    );

    let mut result = Vec::new();
    let data = try_get(&repo, &path).await;
    if let Value::Array(array) = data {
        for mut item in array {
            match item["ns"].take() {
                Value::String(s) if s.starts_with(prefix) => result.push(s),
                _ => (),
            }
        }
    }
    result
}

pub fn base_directories() -> Result<xdg::BaseDirectories, Error> {
    xdg::BaseDirectories::with_prefix("proxmox-backup").map_err(Error::from)
}

/// Convenience helper for better error messages:
pub fn find_xdg_file(
    file_name: impl AsRef<std::path::Path>,
    description: &'static str,
) -> Result<Option<std::path::PathBuf>, Error> {
    let file_name = file_name.as_ref();
    base_directories()
        .map(|base| base.find_config_file(file_name))
        .with_context(|| format!("error searching for {}", description))
}

pub fn place_xdg_file(
    file_name: impl AsRef<std::path::Path>,
    description: &'static str,
) -> Result<std::path::PathBuf, Error> {
    let file_name = file_name.as_ref();
    base_directories()
        .and_then(|base| base.place_config_file(file_name).map_err(Error::from))
        .with_context(|| format!("failed to place {} in xdg home", description))
}
