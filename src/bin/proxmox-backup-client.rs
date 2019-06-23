//#[macro_use]
extern crate proxmox_backup;

use failure::*;
//use std::os::unix::io::AsRawFd;
use chrono::{Local, TimeZone};
use std::path::{Path, PathBuf};
use std::collections::HashMap;

use proxmox_backup::tools;
use proxmox_backup::cli::*;
use proxmox_backup::api_schema::*;
use proxmox_backup::api_schema::router::*;
use proxmox_backup::client::*;
use proxmox_backup::backup::*;
//use proxmox_backup::backup::image_index::*;
//use proxmox_backup::config::datastore;
//use proxmox_backup::pxar::encoder::*;
//use proxmox_backup::backup::datastore::*;

use serde_json::{json, Value};
//use hyper::Body;
use std::sync::Arc;
use regex::Regex;
use xdg::BaseDirectories;

use lazy_static::lazy_static;
use futures::*;
use tokio::sync::mpsc;

lazy_static! {
    static ref BACKUPSPEC_REGEX: Regex = Regex::new(r"^([a-zA-Z0-9_-]+\.(?:pxar|img|conf)):(.+)$").unwrap();

    static ref REPO_URL_SCHEMA: Arc<Schema> = Arc::new(
        StringSchema::new("Repository URL.")
            .format(BACKUP_REPO_URL.clone())
            .max_length(256)
            .into()
    );
}


fn record_repository(repo: &BackupRepository) {

    let base = match BaseDirectories::with_prefix("proxmox-backup") {
        Ok(v) => v,
        _ => return,
    };

    // usually $HOME/.cache/proxmox-backup/repo-list
    let path = match base.place_cache_file("repo-list") {
        Ok(v) => v,
        _ => return,
    };

    let mut data = tools::file_get_json(&path, None).unwrap_or(json!({}));

    let repo = repo.to_string();

    data[&repo] = json!{ data[&repo].as_i64().unwrap_or(0) + 1 };

    let mut map = serde_json::map::Map::new();

    loop {
        let mut max_used = 0;
        let mut max_repo = None;
        for (repo, count) in data.as_object().unwrap() {
            if map.contains_key(repo) { continue; }
            if let Some(count) = count.as_i64() {
                if count > max_used {
                    max_used = count;
                    max_repo = Some(repo);
                }
            }
        }
        if let Some(repo) = max_repo {
            map.insert(repo.to_owned(), json!(max_used));
        } else {
            break;
        }
        if map.len() > 10 { // store max. 10 repos
            break;
        }
    }

    let new_data = json!(map);

    let _ = tools::file_set_contents(path, new_data.to_string().as_bytes(), None);
}

fn complete_repository(_arg: &str, _param: &HashMap<String, String>) -> Vec<String> {

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

    let data = tools::file_get_json(&path, None).unwrap_or(json!({}));

    if let Some(map) = data.as_object() {
        for (repo, _count) in map {
            result.push(repo.to_owned());
        }
    }

    result
}

fn backup_directory<P: AsRef<Path>>(
    client: &BackupClient,
    dir_path: P,
    archive_name: &str,
    chunk_size: Option<usize>,
    all_file_systems: bool,
    verbose: bool,
    crypt_config: Option<Arc<CryptConfig>>,
) -> Result<(), Error> {

    let pxar_stream = PxarBackupStream::open(dir_path.as_ref(), all_file_systems, verbose)?;
    let chunk_stream = ChunkStream::new(pxar_stream, chunk_size);

    let (tx, rx) = mpsc::channel(10); // allow to buffer 10 chunks

    let stream = rx
        .map_err(Error::from)
        .and_then(|x| x); // flatten

    // spawn chunker inside a separate task so that it can run parallel
    tokio::spawn(
        tx.send_all(chunk_stream.then(|r| Ok(r)))
            .map_err(|_| {}).map(|_| ())
    );

    client.upload_stream(archive_name, stream, "dynamic", None, crypt_config).wait()?;

    Ok(())
}

fn backup_image<P: AsRef<Path>>(
    client: &BackupClient,
    image_path: P,
    archive_name: &str,
    image_size: u64,
    chunk_size: Option<usize>,
    _verbose: bool,
    crypt_config: Option<Arc<CryptConfig>>,
) -> Result<(), Error> {

    let path = image_path.as_ref().to_owned();

    let file = tokio::fs::File::open(path).wait()?;

    let stream = tokio::codec::FramedRead::new(file, tokio::codec::BytesCodec::new())
        .map_err(Error::from);

    let stream = FixedChunkStream::new(stream, chunk_size.unwrap_or(4*1024*1024));

    client.upload_stream(archive_name, stream, "fixed", Some(image_size), crypt_config).wait()?;

    Ok(())
}

fn strip_chunked_file_expenstions(list: Vec<String>) -> Vec<String> {

    let mut result = vec![];

    for file in list.into_iter() {
        if file.ends_with(".didx") {
            result.push(file[..file.len()-5].to_owned());
        } else if file.ends_with(".fidx") {
            result.push(file[..file.len()-5].to_owned());
        } else {
            result.push(file); // should not happen
        }
    }

    result
}

/* not used:
fn list_backups(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let repo_url = tools::required_string_param(&param, "repository")?;
    let repo: BackupRepository = repo_url.parse()?;

    let mut client = HttpClient::new(repo.host(), repo.user())?;

    let path = format!("api2/json/admin/datastore/{}/backups", repo.store());

    let result = client.get(&path, None)?;

    record_repository(&repo);

    // fixme: implement and use output formatter instead ..
    let list = result["data"].as_array().unwrap();

    for item in list {

        let id = item["backup-id"].as_str().unwrap();
        let btype = item["backup-type"].as_str().unwrap();
        let epoch = item["backup-time"].as_i64().unwrap();

        let backup_dir = BackupDir::new(btype, id, epoch);

        let files = item["files"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_owned()).collect();
        let files = strip_chunked_file_expenstions(files);

        for filename in files {
            let path = backup_dir.relative_path().to_str().unwrap().to_owned();
            println!("{} | {}/{}", backup_dir.backup_time().format("%c"), path, filename);
        }
    }

    //Ok(result)
    Ok(Value::Null)
}
 */

fn list_backup_groups(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let repo_url = tools::required_string_param(&param, "repository")?;
    let repo: BackupRepository = repo_url.parse()?;

    let client = HttpClient::new(repo.host(), repo.user())?;

    let path = format!("api2/json/admin/datastore/{}/groups", repo.store());

    let mut result = client.get(&path, None).wait()?;

    record_repository(&repo);

    // fixme: implement and use output formatter instead ..
    let list = result["data"].as_array_mut().unwrap();

    list.sort_unstable_by(|a, b| {
        let a_id = a["backup-id"].as_str().unwrap();
        let a_backup_type = a["backup-type"].as_str().unwrap();
        let b_id = b["backup-id"].as_str().unwrap();
        let b_backup_type = b["backup-type"].as_str().unwrap();

        let type_order = a_backup_type.cmp(b_backup_type);
        if type_order == std::cmp::Ordering::Equal {
            a_id.cmp(b_id)
        } else {
            type_order
        }
    });

    for item in list {

        let id = item["backup-id"].as_str().unwrap();
        let btype = item["backup-type"].as_str().unwrap();
        let epoch = item["last-backup"].as_i64().unwrap();
        let last_backup = Local.timestamp(epoch, 0);
        let backup_count = item["backup-count"].as_u64().unwrap();

        let group = BackupGroup::new(btype, id);

        let path = group.group_path().to_str().unwrap().to_owned();

        let files = item["files"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_owned()).collect();
        let files = strip_chunked_file_expenstions(files);

        println!("{:20} | {} | {:5} | {}", path, last_backup.format("%c"),
                 backup_count, tools::join(&files, ' '));
    }

    //Ok(result)
    Ok(Value::Null)
}

fn list_snapshots(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let repo_url = tools::required_string_param(&param, "repository")?;
    let repo: BackupRepository = repo_url.parse()?;

    let path = tools::required_string_param(&param, "group")?;
    let group = BackupGroup::parse(path)?;

    let client = HttpClient::new(repo.host(), repo.user())?;

    let path = format!("api2/json/admin/datastore/{}/snapshots", repo.store());

    let result = client.get(&path, Some(json!({
        "backup-type": group.backup_type(),
        "backup-id": group.backup_id(),
    }))).wait()?;

    record_repository(&repo);

    // fixme: implement and use output formatter instead ..
    let list = result["data"].as_array().unwrap();

    for item in list {

        let id = item["backup-id"].as_str().unwrap();
        let btype = item["backup-type"].as_str().unwrap();
        let epoch = item["backup-time"].as_i64().unwrap();

        let snapshot = BackupDir::new(btype, id, epoch);

        let path = snapshot.relative_path().to_str().unwrap().to_owned();

        let files = item["files"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_owned()).collect();
        let files = strip_chunked_file_expenstions(files);

        println!("{} | {} | {}", path, snapshot.backup_time().format("%c"), tools::join(&files, ' '));
    }

    Ok(Value::Null)
}

fn forget_snapshots(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let repo_url = tools::required_string_param(&param, "repository")?;
    let repo: BackupRepository = repo_url.parse()?;

    let path = tools::required_string_param(&param, "snapshot")?;
    let snapshot = BackupDir::parse(path)?;

    let mut client = HttpClient::new(repo.host(), repo.user())?;

    let path = format!("api2/json/admin/datastore/{}/snapshots", repo.store());

    let result = client.delete(&path, Some(json!({
        "backup-type": snapshot.group().backup_type(),
        "backup-id": snapshot.group().backup_id(),
        "backup-time": snapshot.backup_time().timestamp(),
    }))).wait()?;

    record_repository(&repo);

    Ok(result)
}

fn start_garbage_collection(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let repo_url = tools::required_string_param(&param, "repository")?;
    let repo: BackupRepository = repo_url.parse()?;

    let mut client = HttpClient::new(repo.host(), repo.user())?;

    let path = format!("api2/json/admin/datastore/{}/gc", repo.store());

    let result = client.post(&path, None).wait()?;

    record_repository(&repo);

    Ok(result)
}

fn parse_backupspec(value: &str) -> Result<(&str, &str), Error> {

    if let Some(caps) = BACKUPSPEC_REGEX.captures(value) {
        return Ok((caps.get(1).unwrap().as_str(), caps.get(2).unwrap().as_str()));
    }
    bail!("unable to parse directory specification '{}'", value);
}

fn create_backup(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let repo_url = tools::required_string_param(&param, "repository")?;

    let backupspec_list = tools::required_array_param(&param, "backupspec")?;

    let repo: BackupRepository = repo_url.parse()?;

    let all_file_systems = param["all-file-systems"].as_bool().unwrap_or(false);

    let verbose = param["verbose"].as_bool().unwrap_or(false);

    let chunk_size_opt = param["chunk-size"].as_u64().map(|v| (v*1024) as usize);

    if let Some(size) = chunk_size_opt {
        verify_chunk_size(size)?;
    }

    let keyfile = param["keyfile"].as_str().map(|p| PathBuf::from(p));

    let backup_id = param["host-id"].as_str().unwrap_or(&tools::nodename());

    let mut upload_list = vec![];

    enum BackupType { PXAR, IMAGE, CONFIG };

    for backupspec in backupspec_list {
        let (target, filename) = parse_backupspec(backupspec.as_str().unwrap())?;

        use std::os::unix::fs::FileTypeExt;

        let metadata = match std::fs::metadata(filename) {
            Ok(m) => m,
            Err(err) => bail!("unable to access '{}' - {}", filename, err),
        };
        let file_type = metadata.file_type();

        let extension = Path::new(target).extension().map(|s| s.to_str().unwrap()).unwrap();

        match extension {
            "pxar" => {
                if !file_type.is_dir() {
                    bail!("got unexpected file type (expected directory)");
                }
                upload_list.push((BackupType::PXAR, filename.to_owned(), target.to_owned(), 0));
            }
            "img" => {

                if !(file_type.is_file() || file_type.is_block_device()) {
                    bail!("got unexpected file type (expected file or block device)");
                }

                let size = tools::image_size(&PathBuf::from(filename))?;

                if size == 0 { bail!("got zero-sized file '{}'", filename); }

                upload_list.push((BackupType::IMAGE, filename.to_owned(), target.to_owned(), size));
            }
            "conf" => {
                if !file_type.is_file() {
                    bail!("got unexpected file type (expected regular file)");
                }
                upload_list.push((BackupType::CONFIG, filename.to_owned(), target.to_owned(), metadata.len()));
            }
            _ => {
                bail!("got unknown archive extension '{}'", extension);
            }
        }
    }

    let backup_time = Local.timestamp(Local::now().timestamp(), 0);

    let client = HttpClient::new(repo.host(), repo.user())?;
    record_repository(&repo);

    println!("Starting backup");
    println!("Client name: {}", tools::nodename());
    println!("Start Time: {}", backup_time.to_rfc3339());

    let crypt_config = match keyfile {
        None => None,
        Some(path) => {
            let (key, _) = load_and_decrtypt_key(&path, get_encryption_key_password)?;
            Some(Arc::new(CryptConfig::new(key)?))
        }
    };

    let client = client.start_backup(repo.store(), "host", &backup_id, verbose).wait()?;

    for (backup_type, filename, target, size) in upload_list {
        match backup_type {
            BackupType::CONFIG => {
                println!("Upload config file '{}' to '{:?}' as {}", filename, repo, target);
                client.upload_blob(&filename, &target, crypt_config.clone(), true).wait()?;
            }
            BackupType::PXAR => {
                println!("Upload directory '{}' to '{:?}' as {}", filename, repo, target);
                backup_directory(
                    &client,
                    &filename,
                    &target,
                    chunk_size_opt,
                    all_file_systems,
                    verbose,
                    crypt_config.clone(),
                )?;
            }
            BackupType::IMAGE => {
                println!("Upload image '{}' to '{:?}' as {}", filename, repo, target);
                backup_image(
                    &client,
                    &filename,
                    &target,
                    size,
                    chunk_size_opt,
                    verbose,
                    crypt_config.clone(),
                )?;
            }
        }
    }

    client.finish().wait()?;

    let end_time = Local.timestamp(Local::now().timestamp(), 0);
    let elapsed = end_time.signed_duration_since(backup_time);
    println!("Duration: {}", elapsed);

    println!("End Time: {}", end_time.to_rfc3339());

    Ok(Value::Null)
}

fn complete_backup_source(arg: &str, param: &HashMap<String, String>) -> Vec<String> {

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

fn restore(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let repo_url = tools::required_string_param(&param, "repository")?;
    let repo: BackupRepository = repo_url.parse()?;

    let archive_name = tools::required_string_param(&param, "archive-name")?;

    let mut client = HttpClient::new(repo.host(), repo.user())?;

    record_repository(&repo);

    let path = tools::required_string_param(&param, "snapshot")?;

    let query;

    if path.matches('/').count() == 1 {
        let group = BackupGroup::parse(path)?;

        let path = format!("api2/json/admin/datastore/{}/snapshots", repo.store());
        let result = client.get(&path, Some(json!({
            "backup-type": group.backup_type(),
            "backup-id": group.backup_id(),
        }))).wait()?;

        let list = result["data"].as_array().unwrap();
        if list.len() == 0 {
            bail!("backup group '{}' does not contain any snapshots:", path);
        }

        query = tools::json_object_to_query(json!({
            "backup-type": group.backup_type(),
            "backup-id": group.backup_id(),
            "backup-time": list[0]["backup-time"].as_i64().unwrap(),
            "archive-name": archive_name,
        }))?;
    } else {
        let snapshot = BackupDir::parse(path)?;

        query = tools::json_object_to_query(json!({
            "backup-type": snapshot.group().backup_type(),
            "backup-id": snapshot.group().backup_id(),
            "backup-time": snapshot.backup_time().timestamp(),
            "archive-name": archive_name,
        }))?;
    }

    let target = tools::required_string_param(&param, "target")?;

    if archive_name.ends_with(".pxar") {
        let path = format!("api2/json/admin/datastore/{}/pxar?{}", repo.store(), query);

        println!("DOWNLOAD FILE {} to {}", path, target);

        let target = PathBuf::from(target);
        let writer = PxarDecodeWriter::new(&target, true)?;
        client.download(&path, Box::new(writer)).wait()?;
    } else {
        bail!("unknown file extensions - unable to download '{}'", archive_name);
    }

    Ok(Value::Null)
}

fn prune(
    mut param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let repo_url = tools::required_string_param(&param, "repository")?;
    let repo: BackupRepository = repo_url.parse()?;

    let mut client = HttpClient::new(repo.host(), repo.user())?;

    let path = format!("api2/json/admin/datastore/{}/prune", repo.store());

    param.as_object_mut().unwrap().remove("repository");

    let result = client.post(&path, Some(param)).wait()?;

    record_repository(&repo);

    Ok(result)
}

// like get, but simply ignore errors and return Null instead
fn try_get(repo: &BackupRepository, url: &str) -> Value {

    let client = match HttpClient::new(repo.host(), repo.user()) {
        Ok(v) => v,
        _ => return Value::Null,
    };

    let mut resp = match client.get(url, None).wait() {
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

fn extract_repo(param: &HashMap<String, String>) -> Option<BackupRepository> {

    let repo_url = match param.get("repository") {
        Some(v) => v,
        _ => return None,
    };

    let repo: BackupRepository = match repo_url.parse() {
        Ok(v) => v,
        _ => return None,
    };

    Some(repo)
}

fn complete_backup_group(_arg: &str, param: &HashMap<String, String>) -> Vec<String> {

    let mut result = vec![];

    let repo = match extract_repo(param) {
        Some(v) => v,
        _ => return result,
    };

    let path = format!("api2/json/admin/datastore/{}/groups", repo.store());

    let data = try_get(&repo, &path);

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

fn complete_group_or_snapshot(arg: &str, param: &HashMap<String, String>) -> Vec<String> {

    let mut result = vec![];

     let repo = match extract_repo(param) {
        Some(v) => v,
        _ => return result,
    };

    if arg.matches('/').count() < 2 {
        let groups = complete_backup_group(arg, param);
        for group in groups {
            result.push(group.to_string());
            result.push(format!("{}/", group));
        }
        return result;
    }

    let mut parts = arg.split('/');
    let query = tools::json_object_to_query(json!({
        "backup-type": parts.next().unwrap(),
        "backup-id": parts.next().unwrap(),
    })).unwrap();

    let path = format!("api2/json/admin/datastore/{}/snapshots?{}", repo.store(), query);

    let data = try_get(&repo, &path);

    if let Some(list) = data.as_array() {
        for item in list {
            if let (Some(backup_id), Some(backup_type), Some(backup_time)) =
                (item["backup-id"].as_str(), item["backup-type"].as_str(), item["backup-time"].as_i64())
            {
                let snapshot = BackupDir::new(backup_type, backup_id, backup_time);
                result.push(snapshot.relative_path().to_str().unwrap().to_owned());
            }
        }
    }

    result
}

fn complete_archive_name(_arg: &str, param: &HashMap<String, String>) -> Vec<String> {

    let mut result = vec![];

    let repo = match extract_repo(param) {
        Some(v) => v,
        _ => return result,
    };

    let snapshot = match param.get("snapshot") {
        Some(path) => {
            match BackupDir::parse(path) {
                Ok(v) => v,
                _ => return result,
            }
        }
        _ => return result,
    };

    let query = tools::json_object_to_query(json!({
        "backup-type": snapshot.group().backup_type(),
        "backup-id": snapshot.group().backup_id(),
        "backup-time": snapshot.backup_time().timestamp(),
    })).unwrap();

    let path = format!("api2/json/admin/datastore/{}/files?{}", repo.store(), query);

    let data = try_get(&repo, &path);

    if let Some(list) = data.as_array() {
        for item in list {
            if let Some(filename) = item.as_str() {
                result.push(filename.to_owned());
            }
        }
    }

    strip_chunked_file_expenstions(result)
}

fn complete_chunk_size(_arg: &str, _param: &HashMap<String, String>) -> Vec<String> {

    let mut result = vec![];

    let mut size = 64;
    loop {
        result.push(size.to_string());
        size = size * 2;
        if size > 4096 { break; }
    }

    result
}

fn get_encryption_key_password() -> Result<Vec<u8>, Error> {

    // fixme: implement other input methods

    use std::env::VarError::*;
    match std::env::var("PBS_ENCRYPTION_PASSWORD") {
        Ok(p) => return Ok(p.as_bytes().to_vec()),
        Err(NotUnicode(_)) => bail!("PBS_ENCRYPTION_PASSWORD contains bad characters"),
        Err(NotPresent) => {
            // Try another method
        }
    }

    // If we're on a TTY, query the user for a password
    if crate::tools::tty::stdin_isatty() {
        return Ok(crate::tools::tty::read_password("Encryption Key Password: ")?);
    }

    bail!("no password input mechanism available");
}

fn key_create(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let path = tools::required_string_param(&param, "path")?;
    let path = PathBuf::from(path);

    let kdf = param["kdf"].as_str().unwrap_or("scrypt");

    let key = proxmox::sys::linux::random_data(32)?;

    if kdf == "scrypt" {
        // always read passphrase from tty
        if !crate::tools::tty::stdin_isatty() {
            bail!("unable to read passphrase - no tty");
        }

        let password = crate::tools::tty::read_password("Encryption Key Password: ")?;

        let key_config = encrypt_key_with_passphrase(&key, &password)?;

        store_key_config(&path, false, key_config)?;

        Ok(Value::Null)
    } else if kdf == "none" {
        let created =  Local.timestamp(Local::now().timestamp(), 0);

        store_key_config(&path, false, KeyConfig {
            kdf: None,
            created,
            modified: created,
            data: key,
        })?;

        Ok(Value::Null)
    } else {
        unreachable!();
    }
}

fn key_import_master_pubkey(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let path = tools::required_string_param(&param, "path")?;
    let path = PathBuf::from(path);

    let pem_data = proxmox_backup::tools::file_get_contents(&path)?;

    if let Err(err) = openssl::pkey::PKey::public_key_from_pem(&pem_data) {
        bail!("Unable to decode PEM data - {}", err);
    }

    let base = BaseDirectories::with_prefix("proxmox-backup")?;

    // usually $HOME/.config/proxmox-backup/master-public.pem
    let target_path = base.place_config_file("master-public.pem")?;

    proxmox_backup::tools::file_set_contents(&target_path, &pem_data, None)?;

    println!("Imported public master key to {:?}", target_path);

    Ok(Value::Null)
}

fn key_create_master_key(
    _param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    // we need a TTY to query the new password
    if !crate::tools::tty::stdin_isatty() {
        bail!("unable to create master key - no tty");
    }

    let rsa = openssl::rsa::Rsa::generate(4096)?;
    let pkey = openssl::pkey::PKey::from_rsa(rsa)?;

    let new_pw = String::from_utf8(crate::tools::tty::read_password("Master Key Password: ")?)?;
    let verify_pw = String::from_utf8(crate::tools::tty::read_password("Verify Password: ")?)?;

    if new_pw != verify_pw {
        bail!("Password verification fail!");
    }

    if new_pw.len() < 5 {
        bail!("Password is too short!");
    }

    let pub_key: Vec<u8> = pkey.public_key_to_pem()?;
    let filename_pub = "master-public.pem";
    println!("Writing public master key to {}", filename_pub);
    proxmox_backup::tools::file_set_contents(filename_pub, pub_key.as_slice(), None)?;

    let cipher = openssl::symm::Cipher::aes_256_cbc();
    let priv_key: Vec<u8> = pkey.private_key_to_pem_pkcs8_passphrase(cipher, new_pw.as_bytes())?;

    let filename_priv = "master-private.pem";
    println!("Writing private master key to {}", filename_priv);
    proxmox_backup::tools::file_set_contents(filename_priv, priv_key.as_slice(), None)?;

    Ok(Value::Null)
}

fn key_change_passphrase(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let path = tools::required_string_param(&param, "path")?;
    let path = PathBuf::from(path);

    let kdf = param["kdf"].as_str().unwrap_or("scrypt");

    // we need a TTY to query the new password
    if !crate::tools::tty::stdin_isatty() {
        bail!("unable to change passphrase - no tty");
    }

    let (key, created) = load_and_decrtypt_key(&path, get_encryption_key_password)?;

    if kdf == "scrypt" {

        let new_pw = String::from_utf8(crate::tools::tty::read_password("New Password: ")?)?;
        let verify_pw = String::from_utf8(crate::tools::tty::read_password("Verify Password: ")?)?;

        if new_pw != verify_pw {
            bail!("Password verification fail!");
        }

        if new_pw.len() < 5 {
            bail!("Password is too short!");
        }

        let mut new_key_config = encrypt_key_with_passphrase(&key, new_pw.as_bytes())?;
        new_key_config.created = created; // keep original value

        store_key_config(&path, true, new_key_config)?;

        Ok(Value::Null)
    } else if kdf == "none" {
        let modified =  Local.timestamp(Local::now().timestamp(), 0);

        store_key_config(&path, true, KeyConfig {
            kdf: None,
            created, // keep original value
            modified,
            data: key.to_vec(),
        })?;

        Ok(Value::Null)
    } else {
        unreachable!();
    }
}

fn key_mgmt_cli() -> CliCommandMap {

    let kdf_schema: Arc<Schema> = Arc::new(
        StringSchema::new("Key derivation function. Choose 'none' to store the key unecrypted.")
            .format(Arc::new(ApiStringFormat::Enum(&["scrypt", "none"])))
            .default("scrypt")
            .into()
    );

    let key_create_cmd_def = CliCommand::new(
        ApiMethod::new(
            key_create,
            ObjectSchema::new("Create a new encryption key.")
                .required("path", StringSchema::new("File system path."))
                .optional("kdf", kdf_schema.clone())
        ))
        .arg_param(vec!["path"])
        .completion_cb("path", tools::complete_file_name);

    let key_change_passphrase_cmd_def = CliCommand::new(
        ApiMethod::new(
            key_change_passphrase,
            ObjectSchema::new("Change the passphrase required to decrypt the key.")
                .required("path", StringSchema::new("File system path."))
                .optional("kdf", kdf_schema.clone())
         ))
        .arg_param(vec!["path"])
        .completion_cb("path", tools::complete_file_name);

    let key_create_master_key_cmd_def = CliCommand::new(
        ApiMethod::new(
            key_create_master_key,
            ObjectSchema::new("Create a new 4096 bit RSA master pub/priv key pair.")
        ));

    let key_import_master_pubkey_cmd_def = CliCommand::new(
        ApiMethod::new(
            key_import_master_pubkey,
            ObjectSchema::new("Import a new RSA public key and use it as master key. The key is expected to be in '.pem' format.")
                .required("path", StringSchema::new("File system path."))
        ))
        .arg_param(vec!["path"])
        .completion_cb("path", tools::complete_file_name);

    let cmd_def = CliCommandMap::new()
        .insert("create".to_owned(), key_create_cmd_def.into())
        .insert("create-master-key".to_owned(), key_create_master_key_cmd_def.into())
        .insert("import-master-pubkey".to_owned(), key_import_master_pubkey_cmd_def.into())
        .insert("change-passphrase".to_owned(), key_change_passphrase_cmd_def.into());

    cmd_def
}


fn main() {

    let backup_source_schema: Arc<Schema> = Arc::new(
        StringSchema::new("Backup source specification ([<label>:<path>]).")
            .format(Arc::new(ApiStringFormat::Pattern(&BACKUPSPEC_REGEX)))
            .into()
    );

    let backup_cmd_def = CliCommand::new(
        ApiMethod::new(
            create_backup,
            ObjectSchema::new("Create (host) backup.")
                .required("repository", REPO_URL_SCHEMA.clone())
                .required(
                    "backupspec",
                    ArraySchema::new(
                        "List of backup source specifications ([<label.ext>:<path>] ...)",
                        backup_source_schema,
                    ).min_length(1)
                )
                .optional(
                    "keyfile",
                    StringSchema::new("Path to encryption key. All data will be encrypted using this key."))
                .optional(
                    "verbose",
                    BooleanSchema::new("Verbose output.").default(false))
                .optional(
                    "host-id",
                    StringSchema::new("Use specified ID for the backup group name ('host/<id>'). The default is the system hostname."))
                .optional(
                    "chunk-size",
                    IntegerSchema::new("Chunk size in KB. Must be a power of 2.")
                        .minimum(64)
                        .maximum(4096)
                        .default(4096)
                )
        ))
        .arg_param(vec!["repository", "backupspec"])
        .completion_cb("repository", complete_repository)
        .completion_cb("backupspec", complete_backup_source)
        .completion_cb("keyfile", tools::complete_file_name)
        .completion_cb("chunk-size", complete_chunk_size);

    let list_cmd_def = CliCommand::new(
        ApiMethod::new(
            list_backup_groups,
            ObjectSchema::new("List backup groups.")
                .required("repository", REPO_URL_SCHEMA.clone())
        ))
        .arg_param(vec!["repository"])
        .completion_cb("repository", complete_repository);

    let snapshots_cmd_def = CliCommand::new(
        ApiMethod::new(
            list_snapshots,
            ObjectSchema::new("List backup snapshots.")
                .required("repository", REPO_URL_SCHEMA.clone())
                .required("group", StringSchema::new("Backup group."))
        ))
        .arg_param(vec!["repository", "group"])
        .completion_cb("group", complete_backup_group)
        .completion_cb("repository", complete_repository);

    let forget_cmd_def = CliCommand::new(
        ApiMethod::new(
            forget_snapshots,
            ObjectSchema::new("Forget (remove) backup snapshots.")
                .required("repository", REPO_URL_SCHEMA.clone())
                .required("snapshot", StringSchema::new("Snapshot path."))
        ))
        .arg_param(vec!["repository", "snapshot"])
        .completion_cb("repository", complete_repository)
        .completion_cb("snapshot", complete_group_or_snapshot);

    let garbage_collect_cmd_def = CliCommand::new(
        ApiMethod::new(
            start_garbage_collection,
            ObjectSchema::new("Start garbage collection for a specific repository.")
                .required("repository", REPO_URL_SCHEMA.clone())
        ))
        .arg_param(vec!["repository"])
        .completion_cb("repository", complete_repository);

    let restore_cmd_def = CliCommand::new(
        ApiMethod::new(
            restore,
            ObjectSchema::new("Restore backup repository.")
                .required("repository", REPO_URL_SCHEMA.clone())
                .required("snapshot", StringSchema::new("Group/Snapshot path."))
                .required("archive-name", StringSchema::new("Backup archive name."))
                .required("target", StringSchema::new("Target directory path."))
        ))
        .arg_param(vec!["repository", "snapshot", "archive-name", "target"])
        .completion_cb("repository", complete_repository)
        .completion_cb("snapshot", complete_group_or_snapshot)
        .completion_cb("archive-name", complete_archive_name)
        .completion_cb("target", tools::complete_file_name);

    let prune_cmd_def = CliCommand::new(
        ApiMethod::new(
            prune,
            proxmox_backup::api2::admin::datastore::add_common_prune_prameters(
                ObjectSchema::new("Prune backup repository.")
                    .required("repository", REPO_URL_SCHEMA.clone())
            )
        ))
        .arg_param(vec!["repository"])
        .completion_cb("repository", complete_repository);

    let cmd_def = CliCommandMap::new()
        .insert("backup".to_owned(), backup_cmd_def.into())
        .insert("forget".to_owned(), forget_cmd_def.into())
        .insert("garbage-collect".to_owned(), garbage_collect_cmd_def.into())
        .insert("list".to_owned(), list_cmd_def.into())
        .insert("prune".to_owned(), prune_cmd_def.into())
        .insert("restore".to_owned(), restore_cmd_def.into())
        .insert("snapshots".to_owned(), snapshots_cmd_def.into())
        .insert("key".to_owned(), key_mgmt_cli().into());

    hyper::rt::run(futures::future::lazy(move || {
        run_cli_command(cmd_def.into());
        Ok(())
    }));

}
