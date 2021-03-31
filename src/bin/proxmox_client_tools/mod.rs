//! Shared tools useful for common CLI clients.
use std::collections::HashMap;

use anyhow::{bail, format_err, Context, Error};
use serde_json::{json, Value};
use xdg::BaseDirectories;

use proxmox::{
    api::schema::*,
    tools::fs::file_get_json,
};

use proxmox_backup::api2::access::user::UserWithTokens;
use proxmox_backup::api2::types::*;
use proxmox_backup::backup::BackupDir;
use proxmox_backup::client::*;
use proxmox_backup::tools;

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
    connect_do(repo.host(), repo.port(), repo.auth_id())
        .map_err(|err| format_err!("error building client for repository {} - {}", repo, err))
}

fn connect_do(server: &str, port: u16, auth_id: &Authid) -> Result<HttpClient, Error> {
    let fingerprint = std::env::var(ENV_VAR_PBS_FINGERPRINT).ok();

    use std::env::VarError::*;
    let password = match std::env::var(ENV_VAR_PBS_PASSWORD) {
        Ok(p) => Some(p),
        Err(NotUnicode(_)) => bail!(format!("{} contains bad characters", ENV_VAR_PBS_PASSWORD)),
        Err(NotPresent) => None,
    };

    let options = HttpClientOptions::new_interactive(password, fingerprint);

    HttpClient::new(server, port, auth_id, options)
}

/// like get, but simply ignore errors and return Null instead
pub async fn try_get(repo: &BackupRepository, url: &str) -> Value {

    let fingerprint = std::env::var(ENV_VAR_PBS_FINGERPRINT).ok();
    let password = std::env::var(ENV_VAR_PBS_PASSWORD).ok();

    // ticket cache, but no questions asked
    let options = HttpClientOptions::new_interactive(password, fingerprint)
        .interactive(false);

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
    proxmox_backup::tools::runtime::main(async { complete_backup_group_do(param).await })
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
    proxmox_backup::tools::runtime::main(async { complete_group_or_snapshot_do(arg, param).await })
}

pub async fn complete_group_or_snapshot_do(arg: &str, param: &HashMap<String, String>) -> Vec<String> {

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
    proxmox_backup::tools::runtime::main(async { complete_backup_snapshot_do(param).await })
}

pub async fn complete_backup_snapshot_do(param: &HashMap<String, String>) -> Vec<String> {

    let mut result = vec![];

    let repo = match extract_repository_from_map(param) {
        Some(v) => v,
        _ => return result,
    };

    let path = format!("api2/json/admin/datastore/{}/snapshots", repo.store());

    let data = try_get(&repo, &path).await;

    if let Some(list) = data.as_array() {
        for item in list {
            if let (Some(backup_id), Some(backup_type), Some(backup_time)) =
                (item["backup-id"].as_str(), item["backup-type"].as_str(), item["backup-time"].as_i64())
            {
                if let Ok(snapshot) = BackupDir::new(backup_type, backup_id, backup_time) {
                    result.push(snapshot.relative_path().to_str().unwrap().to_owned());
                }
            }
        }
    }

    result
}

pub fn complete_server_file_name(_arg: &str, param: &HashMap<String, String>) -> Vec<String> {
    proxmox_backup::tools::runtime::main(async { complete_server_file_name_do(param).await })
}

pub async fn complete_server_file_name_do(param: &HashMap<String, String>) -> Vec<String> {

    let mut result = vec![];

    let repo = match extract_repository_from_map(param) {
        Some(v) => v,
        _ => return result,
    };

    let snapshot: BackupDir = match param.get("snapshot") {
        Some(path) => {
            match path.parse() {
                Ok(v) => v,
                _ => return result,
            }
        }
        _ => return result,
    };

    let query = tools::json_object_to_query(json!({
        "backup-type": snapshot.group().backup_type(),
        "backup-id": snapshot.group().backup_id(),
        "backup-time": snapshot.backup_time(),
    })).unwrap();

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
        .map(|v| tools::format::strip_server_file_extension(&v))
        .collect()
}

pub fn complete_pxar_archive_name(arg: &str, param: &HashMap<String, String>) -> Vec<String> {
    complete_server_file_name(arg, param)
        .iter()
        .filter_map(|name| {
            if name.ends_with(".pxar.didx") {
                Some(tools::format::strip_server_file_extension(name))
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
                Some(tools::format::strip_server_file_extension(name))
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
        if size > 4096 { break; }
    }

    result
}

pub fn complete_auth_id(_arg: &str, param: &HashMap<String, String>) -> Vec<String> {
    proxmox_backup::tools::runtime::main(async { complete_auth_id_do(param).await })
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

    let data = file_get_json(&path, None).unwrap_or_else(|_| json!({}));

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

    let files = tools::complete_file_name(data[1], param);

    for file in files {
        result.push(format!("{}:{}", data[0], file));
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
