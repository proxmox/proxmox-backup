//#[macro_use]
extern crate proxmox_backup;

use failure::*;
use nix::unistd::{fork, ForkResult, pipe};
use std::os::unix::io::RawFd;
use chrono::{Local, Utc, TimeZone};
use std::path::{Path, PathBuf};
use std::collections::{HashSet, HashMap};
use std::ffi::OsStr;
use std::io::{Read, Write, Seek, SeekFrom};
use std::os::unix::fs::OpenOptionsExt;

use proxmox::tools::fs::{file_get_contents, file_get_json, file_set_contents, image_size};

use proxmox_backup::tools;
use proxmox_backup::cli::*;
use proxmox_backup::api2::types::*;
use proxmox_backup::api_schema::*;
use proxmox_backup::api_schema::router::*;
use proxmox_backup::client::*;
use proxmox_backup::backup::*;
use proxmox_backup::pxar::{ self, catalog::* };

//use proxmox_backup::backup::image_index::*;
//use proxmox_backup::config::datastore;
//use proxmox_backup::pxar::encoder::*;
//use proxmox_backup::backup::datastore::*;

use serde_json::{json, Value};
//use hyper::Body;
use std::sync::{Arc, Mutex};
use regex::Regex;
use xdg::BaseDirectories;

use lazy_static::lazy_static;
use futures::*;
use tokio::sync::mpsc;

lazy_static! {
    static ref BACKUPSPEC_REGEX: Regex = Regex::new(r"^([a-zA-Z0-9_-]+\.(?:pxar|img|conf|log)):(.+)$").unwrap();

    static ref REPO_URL_SCHEMA: Arc<Schema> = Arc::new(
        StringSchema::new("Repository URL.")
            .format(BACKUP_REPO_URL.clone())
            .max_length(256)
            .into()
    );
}


fn get_default_repository() -> Option<String> {
    std::env::var("PBS_REPOSITORY").ok()
}

fn extract_repository_from_value(
    param: &Value,
) -> Result<BackupRepository, Error> {

    let repo_url = param["repository"]
        .as_str()
        .map(String::from)
        .or_else(get_default_repository)
        .ok_or_else(|| format_err!("unable to get (default) repository"))?;

    let repo: BackupRepository = repo_url.parse()?;

    Ok(repo)
}

fn extract_repository_from_map(
    param: &HashMap<String, String>,
) -> Option<BackupRepository> {

    param.get("repository")
        .map(String::from)
        .or_else(get_default_repository)
        .and_then(|repo_url| repo_url.parse::<BackupRepository>().ok())
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

    let mut data = file_get_json(&path, None).unwrap_or_else(|_| json!({}));

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

    let _ = file_set_contents(path, new_data.to_string().as_bytes(), None);
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

    let data = file_get_json(&path, None).unwrap_or_else(|_| json!({}));

    if let Some(map) = data.as_object() {
        for (repo, _count) in map {
            result.push(repo.to_owned());
        }
    }

    result
}

fn compute_file_csum(file: &mut std::fs::File) -> Result<([u8; 32], u64), Error> {

    file.seek(SeekFrom::Start(0))?;

    let mut hasher = openssl::sha::Sha256::new();
    let mut buffer = proxmox::tools::vec::undefined(256*1024);
    let mut size: u64 = 0;

    loop {
        let count = match file.read(&mut buffer) {
            Ok(count) => count,
            Err(ref err) if err.kind() == std::io::ErrorKind::Interrupted => { continue; }
            Err(err) => return Err(err.into()),
        };
        if count == 0 {
            break;
        }
        size += count as u64;
        hasher.update(&buffer[..count]);
    }

    let csum = hasher.finish();

    Ok((csum, size))
}


async fn backup_directory<P: AsRef<Path>>(
    client: &BackupWriter,
    dir_path: P,
    archive_name: &str,
    chunk_size: Option<usize>,
    device_set: Option<HashSet<u64>>,
    verbose: bool,
    skip_lost_and_found: bool,
    crypt_config: Option<Arc<CryptConfig>>,
    catalog: Arc<Mutex<CatalogWriter<SenderWriter>>>,
) -> Result<BackupStats, Error> {

    let pxar_stream = PxarBackupStream::open(dir_path.as_ref(), device_set, verbose, skip_lost_and_found, catalog)?;
    let mut chunk_stream = ChunkStream::new(pxar_stream, chunk_size);

    let (mut tx, rx) = mpsc::channel(10); // allow to buffer 10 chunks

    let stream = rx
        .map_err(Error::from);

    // spawn chunker inside a separate task so that it can run parallel
    tokio::spawn(async move {
        let _ = tx.send_all(&mut chunk_stream).await;
    });

    let stats = client
        .upload_stream(archive_name, stream, "dynamic", None, crypt_config)
        .await?;

    Ok(stats)
}

async fn backup_image<P: AsRef<Path>>(
    client: &BackupWriter,
    image_path: P,
    archive_name: &str,
    image_size: u64,
    chunk_size: Option<usize>,
    _verbose: bool,
    crypt_config: Option<Arc<CryptConfig>>,
) -> Result<BackupStats, Error> {

    let path = image_path.as_ref().to_owned();

    let file = tokio::fs::File::open(path).await?;

    let stream = tokio::codec::FramedRead::new(file, tokio::codec::BytesCodec::new())
        .map_err(Error::from);

    let stream = FixedChunkStream::new(stream, chunk_size.unwrap_or(4*1024*1024));

    let stats = client
        .upload_stream(archive_name, stream, "fixed", Some(image_size), crypt_config)
        .await?;

    Ok(stats)
}

fn strip_server_file_expenstion(name: &str) -> String {

    if name.ends_with(".didx") || name.ends_with(".fidx") || name.ends_with(".blob") {
        name[..name.len()-5].to_owned()
    } else {
        name.to_owned() // should not happen
    }
}

fn list_backup_groups(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let repo = extract_repository_from_value(&param)?;

    let client = HttpClient::new(repo.host(), repo.user(), None)?;

    let path = format!("api2/json/admin/datastore/{}/groups", repo.store());

    let mut result = async_main(async move {
        client.get(&path, None).await
    })?;

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

    let output_format = param["output-format"].as_str().unwrap_or("text").to_owned();

    let mut result = vec![];

    for item in list {

        let id = item["backup-id"].as_str().unwrap();
        let btype = item["backup-type"].as_str().unwrap();
        let epoch = item["last-backup"].as_i64().unwrap();
        let last_backup = Utc.timestamp(epoch, 0);
        let backup_count = item["backup-count"].as_u64().unwrap();

        let group = BackupGroup::new(btype, id);

        let path = group.group_path().to_str().unwrap().to_owned();

        let files = item["files"].as_array().unwrap().iter()
            .map(|v| strip_server_file_expenstion(v.as_str().unwrap())).collect();

        if output_format == "text" {
            println!(
                "{:20} | {} | {:5} | {}",
                path,
                BackupDir::backup_time_to_string(last_backup),
                backup_count,
                tools::join(&files, ' '),
            );
        } else {
            result.push(json!({
                "backup-type": btype,
                "backup-id": id,
                "last-backup": epoch,
                "backup-count": backup_count,
                "files": files,
            }));
        }
    }

    if output_format != "text" { format_and_print_result(&result.into(), &output_format); }

    Ok(Value::Null)
}

fn list_snapshots(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let repo = extract_repository_from_value(&param)?;

    let output_format = param["output-format"].as_str().unwrap_or("text").to_owned();

    let client = HttpClient::new(repo.host(), repo.user(), None)?;

    let path = format!("api2/json/admin/datastore/{}/snapshots", repo.store());

    let mut args = json!({});
    if let Some(path) = param["group"].as_str() {
        let group = BackupGroup::parse(path)?;
        args["backup-type"] = group.backup_type().into();
        args["backup-id"] = group.backup_id().into();
    }

    let result = async_main(async move {
        client.get(&path, Some(args)).await
    })?;

    record_repository(&repo);

    let list = result["data"].as_array().unwrap();

    let mut result = vec![];

    for item in list {

        let id = item["backup-id"].as_str().unwrap();
        let btype = item["backup-type"].as_str().unwrap();
        let epoch = item["backup-time"].as_i64().unwrap();

        let snapshot = BackupDir::new(btype, id, epoch);

        let path = snapshot.relative_path().to_str().unwrap().to_owned();

        let files = item["files"].as_array().unwrap().iter()
            .map(|v|  strip_server_file_expenstion(v.as_str().unwrap())).collect();

        if output_format == "text" {
            let size_str = if let Some(size) = item["size"].as_u64() {
                size.to_string()
            } else {
                String::from("-")
            };
            println!("{} | {} | {}", path, size_str, tools::join(&files, ' '));
        } else {
            let mut data = json!({
                "backup-type": btype,
                "backup-id": id,
                "backup-time": epoch,
                "files": files,
            });
            if let Some(size) = item["size"].as_u64() {
                data["size"] = size.into();
            }
            result.push(data);
        }
    }

    if output_format != "text" { format_and_print_result(&result.into(), &output_format); }

    Ok(Value::Null)
}

fn forget_snapshots(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let repo = extract_repository_from_value(&param)?;

    let path = tools::required_string_param(&param, "snapshot")?;
    let snapshot = BackupDir::parse(path)?;

    let mut client = HttpClient::new(repo.host(), repo.user(), None)?;

    let path = format!("api2/json/admin/datastore/{}/snapshots", repo.store());

    let result = async_main(async move {
        client.delete(&path, Some(json!({
            "backup-type": snapshot.group().backup_type(),
            "backup-id": snapshot.group().backup_id(),
            "backup-time": snapshot.backup_time().timestamp(),
        }))).await
    })?;

    record_repository(&repo);

    Ok(result)
}

fn api_login(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let repo = extract_repository_from_value(&param)?;

    let client = HttpClient::new(repo.host(), repo.user(), None)?;
    async_main(async move { client.login().await })?;

    record_repository(&repo);

    Ok(Value::Null)
}

fn api_logout(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let repo = extract_repository_from_value(&param)?;

    delete_ticket_info(repo.host(), repo.user())?;

    Ok(Value::Null)
}

fn dump_catalog(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let repo = extract_repository_from_value(&param)?;

    let path = tools::required_string_param(&param, "snapshot")?;
    let snapshot = BackupDir::parse(path)?;

    let keyfile = param["keyfile"].as_str().map(PathBuf::from);

    let crypt_config = match keyfile {
        None => None,
        Some(path) => {
            let (key, _) = load_and_decrtypt_key(&path, &get_encryption_key_password)?;
            Some(Arc::new(CryptConfig::new(key)?))
        }
    };

    let client = HttpClient::new(repo.host(), repo.user(), None)?;

    async_main(async move {
        let client = BackupReader::start(
            client,
            crypt_config.clone(),
            repo.store(),
            &snapshot.group().backup_type(),
            &snapshot.group().backup_id(),
            snapshot.backup_time(),
            true,
        ).await?;

        let manifest = client.download_manifest().await?;

        let index = client.download_dynamic_index(&manifest, CATALOG_NAME).await?;

        let most_used = index.find_most_used_chunks(8);

        let chunk_reader = RemoteChunkReader::new(client.clone(), crypt_config, most_used);

        let mut reader = BufferedDynamicReader::new(index, chunk_reader);

        let mut catalogfile = std::fs::OpenOptions::new()
            .write(true)
            .read(true)
            .custom_flags(libc::O_TMPFILE)
            .open("/tmp")?;

        std::io::copy(&mut reader, &mut catalogfile)
            .map_err(|err| format_err!("unable to download catalog - {}", err))?;

        catalogfile.seek(SeekFrom::Start(0))?;

        let mut catalog_reader = CatalogReader::new(catalogfile);

        catalog_reader.dump()?;

        record_repository(&repo);

        Ok::<(), Error>(())
    })?;

    Ok(Value::Null)
}

fn list_snapshot_files(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let repo = extract_repository_from_value(&param)?;

    let path = tools::required_string_param(&param, "snapshot")?;
    let snapshot = BackupDir::parse(path)?;

    let output_format = param["output-format"].as_str().unwrap_or("text").to_owned();

    let client = HttpClient::new(repo.host(), repo.user(), None)?;

    let path = format!("api2/json/admin/datastore/{}/files", repo.store());

    let mut result = async_main(async move {
        client.get(&path, Some(json!({
            "backup-type": snapshot.group().backup_type(),
            "backup-id": snapshot.group().backup_id(),
            "backup-time": snapshot.backup_time().timestamp(),
        }))).await
    })?;

    record_repository(&repo);

    let list: Value = result["data"].take();

    if output_format == "text" {
        for item in list.as_array().unwrap().iter() {
            println!(
                "{} {}",
                strip_server_file_expenstion(item["filename"].as_str().unwrap()),
                item["size"].as_u64().unwrap_or(0),
            );
        }
    } else {
        format_and_print_result(&list, &output_format);
    }

    Ok(Value::Null)
}

fn start_garbage_collection(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let repo = extract_repository_from_value(&param)?;

    let mut client = HttpClient::new(repo.host(), repo.user(), None)?;

    let path = format!("api2/json/admin/datastore/{}/gc", repo.store());

    let result = async_main(async move { client.post(&path, None).await })?;

    record_repository(&repo);

    Ok(result)
}

fn parse_backupspec(value: &str) -> Result<(&str, &str), Error> {

    if let Some(caps) = BACKUPSPEC_REGEX.captures(value) {
        return Ok((caps.get(1).unwrap().as_str(), caps.get(2).unwrap().as_str()));
    }
    bail!("unable to parse directory specification '{}'", value);
}

fn spawn_catalog_upload(
    client: Arc<BackupWriter>,
    crypt_config: Option<Arc<CryptConfig>>,
) -> Result<
        (
            Arc<Mutex<CatalogWriter<SenderWriter>>>,
            tokio::sync::oneshot::Receiver<Result<BackupStats, Error>>
        ), Error>
{
    let (catalog_tx, catalog_rx) = mpsc::channel(10); // allow to buffer 10 writes
    let catalog_stream = catalog_rx.map_err(Error::from);
    let catalog_chunk_size = 512*1024;
    let catalog_chunk_stream = ChunkStream::new(catalog_stream, Some(catalog_chunk_size));

    let catalog = Arc::new(Mutex::new(CatalogWriter::new(SenderWriter::new(catalog_tx))?));

    let (catalog_result_tx, catalog_result_rx) = tokio::sync::oneshot::channel();

    tokio::spawn(async move {
        let catalog_upload_result = client
            .upload_stream(CATALOG_NAME, catalog_chunk_stream, "dynamic", None, crypt_config)
            .await;

        if let Err(ref err) = catalog_upload_result {
            eprintln!("catalog upload error - {}", err);
            client.cancel();
        }

        let _ = catalog_result_tx.send(catalog_upload_result);
    });

    Ok((catalog, catalog_result_rx))
}

fn create_backup(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let repo = extract_repository_from_value(&param)?;

    let backupspec_list = tools::required_array_param(&param, "backupspec")?;

    let all_file_systems = param["all-file-systems"].as_bool().unwrap_or(false);

    let skip_lost_and_found = param["skip-lost-and-found"].as_bool().unwrap_or(false);

    let verbose = param["verbose"].as_bool().unwrap_or(false);

    let backup_time_opt = param["backup-time"].as_i64();

    let chunk_size_opt = param["chunk-size"].as_u64().map(|v| (v*1024) as usize);

    if let Some(size) = chunk_size_opt {
        verify_chunk_size(size)?;
    }

    let keyfile = param["keyfile"].as_str().map(PathBuf::from);

    let backup_id = param["backup-id"].as_str().unwrap_or(&proxmox::tools::nodename());

    let backup_type = param["backup-type"].as_str().unwrap_or("host");

    let include_dev = param["include-dev"].as_array();

    let mut devices = if all_file_systems { None } else { Some(HashSet::new()) };

    if let Some(include_dev) = include_dev {
        if all_file_systems {
            bail!("option 'all-file-systems' conflicts with option 'include-dev'");
        }

        let mut set = HashSet::new();
        for path in include_dev {
            let path = path.as_str().unwrap();
            let stat = nix::sys::stat::stat(path)
                .map_err(|err| format_err!("fstat {:?} failed - {}", path, err))?;
            set.insert(stat.st_dev);
        }
        devices = Some(set);
    }

    let mut upload_list = vec![];

    enum BackupType { PXAR, IMAGE, CONFIG, LOGFILE };

    let mut upload_catalog = false;

    for backupspec in backupspec_list {
        let (target, filename) = parse_backupspec(backupspec.as_str().unwrap())?;

        use std::os::unix::fs::FileTypeExt;

        let metadata = std::fs::metadata(filename)
            .map_err(|err| format_err!("unable to access '{}' - {}", filename, err))?;
        let file_type = metadata.file_type();

        let extension = target.rsplit('.').next()
            .ok_or_else(|| format_err!("missing target file extenion '{}'", target))?;

        match extension {
            "pxar" => {
                if !file_type.is_dir() {
                    bail!("got unexpected file type (expected directory)");
                }
                upload_list.push((BackupType::PXAR, filename.to_owned(), format!("{}.didx", target), 0));
                upload_catalog = true;
            }
            "img" => {

                if !(file_type.is_file() || file_type.is_block_device()) {
                    bail!("got unexpected file type (expected file or block device)");
                }

                let size = image_size(&PathBuf::from(filename))?;

                if size == 0 { bail!("got zero-sized file '{}'", filename); }

                upload_list.push((BackupType::IMAGE, filename.to_owned(), format!("{}.fidx", target), size));
            }
            "conf" => {
                if !file_type.is_file() {
                    bail!("got unexpected file type (expected regular file)");
                }
                upload_list.push((BackupType::CONFIG, filename.to_owned(), format!("{}.blob", target), metadata.len()));
            }
            "log" => {
                if !file_type.is_file() {
                    bail!("got unexpected file type (expected regular file)");
                }
                upload_list.push((BackupType::LOGFILE, filename.to_owned(), format!("{}.blob", target), metadata.len()));
            }
            _ => {
                bail!("got unknown archive extension '{}'", extension);
            }
        }
    }

    let backup_time = Utc.timestamp(backup_time_opt.unwrap_or_else(|| Utc::now().timestamp()), 0);

    let client = HttpClient::new(repo.host(), repo.user(), None)?;
    record_repository(&repo);

    println!("Starting backup: {}/{}/{}", backup_type, backup_id, BackupDir::backup_time_to_string(backup_time));

    println!("Client name: {}", proxmox::tools::nodename());

    let start_time = Local::now();

    println!("Starting protocol: {}", start_time.to_rfc3339_opts(chrono::SecondsFormat::Secs, false));

    let (crypt_config, rsa_encrypted_key) = match keyfile {
        None => (None, None),
        Some(path) => {
            let (key, created) = load_and_decrtypt_key(&path, &get_encryption_key_password)?;

            let crypt_config = CryptConfig::new(key)?;

            let path = master_pubkey_path()?;
            if path.exists() {
                let pem_data = file_get_contents(&path)?;
                let rsa = openssl::rsa::Rsa::public_key_from_pem(&pem_data)?;
                let enc_key = crypt_config.generate_rsa_encoded_key(rsa, created)?;
                (Some(Arc::new(crypt_config)), Some(enc_key))
            } else {
                (Some(Arc::new(crypt_config)), None)
            }
        }
    };

    async_main(async move {
        let client = BackupWriter::start(
            client,
            repo.store(),
            backup_type,
            &backup_id,
            backup_time,
            verbose,
        ).await?;

        let snapshot = BackupDir::new(backup_type, backup_id, backup_time.timestamp());
        let mut manifest = BackupManifest::new(snapshot);

        let (catalog, catalog_result_rx) = spawn_catalog_upload(client.clone(), crypt_config.clone())?;

        for (backup_type, filename, target, size) in upload_list {
            match backup_type {
                BackupType::CONFIG => {
                    println!("Upload config file '{}' to '{:?}' as {}", filename, repo, target);
                    let stats = client
                        .upload_blob_from_file(&filename, &target, crypt_config.clone(), true)
                        .await?;
                    manifest.add_file(target, stats.size, stats.csum);
                }
                BackupType::LOGFILE => { // fixme: remove - not needed anymore ?
                    println!("Upload log file '{}' to '{:?}' as {}", filename, repo, target);
                    let stats = client
                        .upload_blob_from_file(&filename, &target, crypt_config.clone(), true)
                        .await?;
                    manifest.add_file(target, stats.size, stats.csum);
                }
                BackupType::PXAR => {
                    println!("Upload directory '{}' to '{:?}' as {}", filename, repo, target);
                    catalog.lock().unwrap().start_directory(std::ffi::CString::new(target.as_str())?.as_c_str())?;
                    let stats = backup_directory(
                        &client,
                        &filename,
                        &target,
                        chunk_size_opt,
                        devices.clone(),
                        verbose,
                        skip_lost_and_found,
                        crypt_config.clone(),
                        catalog.clone(),
                    ).await?;
                    manifest.add_file(target, stats.size, stats.csum);
                    catalog.lock().unwrap().end_directory()?;
                }
                BackupType::IMAGE => {
                    println!("Upload image '{}' to '{:?}' as {}", filename, repo, target);
                    let stats = backup_image(
                        &client,
                        &filename,
                        &target,
                        size,
                        chunk_size_opt,
                        verbose,
                        crypt_config.clone(),
                    ).await?;
                    manifest.add_file(target, stats.size, stats.csum);
                }
            }
        }

        // finalize and upload catalog
        if upload_catalog {
            let mutex = Arc::try_unwrap(catalog)
                .map_err(|_| format_err!("unable to get catalog (still used)"))?;
            let mut catalog = mutex.into_inner().unwrap();

            catalog.finish()?;

            drop(catalog); // close upload stream

            let stats = catalog_result_rx.await??;

            manifest.add_file(CATALOG_NAME.to_owned(), stats.size, stats.csum);
        }

        if let Some(rsa_encrypted_key) = rsa_encrypted_key {
            let target = "rsa-encrypted.key";
            println!("Upload RSA encoded key to '{:?}' as {}", repo, target);
            let stats = client
                .upload_blob_from_data(rsa_encrypted_key, target, None, false, false)
                .await?;
            manifest.add_file(format!("{}.blob", target), stats.size, stats.csum);

            // openssl rsautl -decrypt -inkey master-private.pem -in rsa-encrypted.key -out t
            /*
            let mut buffer2 = vec![0u8; rsa.size() as usize];
            let pem_data = file_get_contents("master-private.pem")?;
            let rsa = openssl::rsa::Rsa::private_key_from_pem(&pem_data)?;
            let len = rsa.private_decrypt(&buffer, &mut buffer2, openssl::rsa::Padding::PKCS1)?;
            println!("TEST {} {:?}", len, buffer2);
             */
        }

        // create manifest (index.json)
        let manifest = manifest.into_json();

        println!("Upload index.json to '{:?}'", repo);
        let manifest = serde_json::to_string_pretty(&manifest)?.into();
        client
            .upload_blob_from_data(manifest, MANIFEST_BLOB_NAME, crypt_config.clone(), true, true)
            .await?;

        client.finish().await?;

        let end_time = Local::now();
        let elapsed = end_time.signed_duration_since(start_time);
        println!("Duration: {}", elapsed);

        println!("End Time: {}", end_time.to_rfc3339_opts(chrono::SecondsFormat::Secs, false));

        Ok(Value::Null)
    })
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
    async_main(restore_do(param))
}

fn dump_image<W: Write>(
    client: Arc<BackupReader>,
    crypt_config: Option<Arc<CryptConfig>>,
    index: FixedIndexReader,
    mut writer: W,
    verbose: bool,
) -> Result<(), Error> {

    let most_used = index.find_most_used_chunks(8);

    let mut chunk_reader = RemoteChunkReader::new(client.clone(), crypt_config, most_used);

    // Note: we avoid using BufferedFixedReader, because that add an additional buffer/copy
    // and thus slows down reading. Instead, directly use RemoteChunkReader
    let mut per = 0;
    let mut bytes = 0;
    let start_time = std::time::Instant::now();

    for pos in 0..index.index_count() {
        let digest = index.index_digest(pos).unwrap();
        let raw_data = chunk_reader.read_chunk(&digest)?;
        writer.write_all(&raw_data)?;
        bytes += raw_data.len();
        if verbose {
            let next_per = ((pos+1)*100)/index.index_count();
            if per != next_per {
                eprintln!("progress {}% (read {} bytes, duration {} sec)",
                          next_per, bytes, start_time.elapsed().as_secs());
                per = next_per;
            }
        }
    }

    let end_time = std::time::Instant::now();
    let elapsed = end_time.duration_since(start_time);
    eprintln!("restore image complete (bytes={}, duration={:.2}s, speed={:.2}MB/s)",
              bytes,
              elapsed.as_secs_f64(),
              bytes as f64/(1024.0*1024.0*elapsed.as_secs_f64())
    );


    Ok(())
}

async fn restore_do(param: Value) -> Result<Value, Error> {
    let repo = extract_repository_from_value(&param)?;

    let verbose = param["verbose"].as_bool().unwrap_or(false);

    let allow_existing_dirs = param["allow-existing-dirs"].as_bool().unwrap_or(false);

    let archive_name = tools::required_string_param(&param, "archive-name")?;

    let client = HttpClient::new(repo.host(), repo.user(), None)?;

    record_repository(&repo);

    let path = tools::required_string_param(&param, "snapshot")?;

    let (backup_type, backup_id, backup_time) = if path.matches('/').count() == 1 {
        let group = BackupGroup::parse(path)?;

        let path = format!("api2/json/admin/datastore/{}/snapshots", repo.store());
        let result = client.get(&path, Some(json!({
            "backup-type": group.backup_type(),
            "backup-id": group.backup_id(),
        }))).await?;

        let list = result["data"].as_array().unwrap();
        if list.is_empty() {
            bail!("backup group '{}' does not contain any snapshots:", path);
        }

        let epoch = list[0]["backup-time"].as_i64().unwrap();
        let backup_time = Utc.timestamp(epoch, 0);
        (group.backup_type().to_owned(), group.backup_id().to_owned(), backup_time)
    } else {
        let snapshot = BackupDir::parse(path)?;
        (snapshot.group().backup_type().to_owned(), snapshot.group().backup_id().to_owned(), snapshot.backup_time())
    };

    let target = tools::required_string_param(&param, "target")?;
    let target = if target == "-" { None } else { Some(target) };

    let keyfile = param["keyfile"].as_str().map(PathBuf::from);

    let crypt_config = match keyfile {
        None => None,
        Some(path) => {
            let (key, _) = load_and_decrtypt_key(&path, &get_encryption_key_password)?;
            Some(Arc::new(CryptConfig::new(key)?))
        }
    };

    let server_archive_name = if archive_name.ends_with(".pxar") {
        format!("{}.didx", archive_name)
    } else if archive_name.ends_with(".img") {
        format!("{}.fidx", archive_name)
    } else {
        format!("{}.blob", archive_name)
    };

    let client = BackupReader::start(
        client,
        crypt_config.clone(),
        repo.store(),
        &backup_type,
        &backup_id,
        backup_time,
        true,
    ).await?;

    let tmpfile = std::fs::OpenOptions::new()
        .write(true)
        .read(true)
        .custom_flags(libc::O_TMPFILE)
        .open("/tmp")?;

    let manifest = client.download_manifest().await?;

    if server_archive_name == MANIFEST_BLOB_NAME {
        let backup_index_data = manifest.into_json().to_string();
        if let Some(target) = target {
            file_set_contents(target, backup_index_data.as_bytes(), None)?;
        } else {
            let stdout = std::io::stdout();
            let mut writer = stdout.lock();
            writer.write_all(backup_index_data.as_bytes())
                .map_err(|err| format_err!("unable to pipe data - {}", err))?;
        }

    } else if server_archive_name.ends_with(".blob") {
        let mut tmpfile = client.download(&server_archive_name, tmpfile).await?;

        let (csum, size) = compute_file_csum(&mut tmpfile)?;
        manifest.verify_file(&server_archive_name, &csum, size)?;

        tmpfile.seek(SeekFrom::Start(0))?;
        let mut reader = DataBlobReader::new(tmpfile, crypt_config)?;

        if let Some(target) = target {
           let mut writer = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .create_new(true)
                .open(target)
                .map_err(|err| format_err!("unable to create target file {:?} - {}", target, err))?;
            std::io::copy(&mut reader, &mut writer)?;
        } else {
            let stdout = std::io::stdout();
            let mut writer = stdout.lock();
            std::io::copy(&mut reader, &mut writer)
                .map_err(|err| format_err!("unable to pipe data - {}", err))?;
        }

    } else if server_archive_name.ends_with(".didx") {

        let index = client.download_dynamic_index(&manifest, &server_archive_name).await?;

        let most_used = index.find_most_used_chunks(8);

        let chunk_reader = RemoteChunkReader::new(client.clone(), crypt_config, most_used);

        let mut reader = BufferedDynamicReader::new(index, chunk_reader);

        if let Some(target) = target {

            let feature_flags = pxar::flags::DEFAULT;
            let mut decoder = pxar::SequentialDecoder::new(&mut reader, feature_flags, |path| {
                if verbose {
                    eprintln!("{:?}", path);
                }
                Ok(())
            });
            decoder.set_allow_existing_dirs(allow_existing_dirs);

            decoder.restore(Path::new(target), &Vec::new())?;
        } else {
            let mut writer = std::fs::OpenOptions::new()
                .write(true)
                .open("/dev/stdout")
                .map_err(|err| format_err!("unable to open /dev/stdout - {}", err))?;

            std::io::copy(&mut reader, &mut writer)
                .map_err(|err| format_err!("unable to pipe data - {}", err))?;
        }
    } else if server_archive_name.ends_with(".fidx") {
        let tmpfile = client.download(&server_archive_name, tmpfile).await?;

        let index = FixedIndexReader::new(tmpfile)
            .map_err(|err| format_err!("unable to read fixed index '{}' - {}", archive_name, err))?;

        // Note: do not use values stored in index (not trusted) - instead, computed them again
        let (csum, size) = index.compute_csum();
        manifest.verify_file(&server_archive_name, &csum, size)?;

        let mut writer = if let Some(target) = target {
            std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .create_new(true)
                .open(target)
                .map_err(|err| format_err!("unable to create target file {:?} - {}", target, err))?
        } else {
            std::fs::OpenOptions::new()
                .write(true)
                .open("/dev/stdout")
                .map_err(|err| format_err!("unable to open /dev/stdout - {}", err))?
        };

        dump_image(client.clone(), crypt_config.clone(), index, &mut writer, verbose)?;

     } else {
        bail!("unknown archive file extension (expected .pxar of .img)");
    }

    Ok(Value::Null)
}

fn upload_log(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let logfile = tools::required_string_param(&param, "logfile")?;
    let repo = extract_repository_from_value(&param)?;

    let snapshot = tools::required_string_param(&param, "snapshot")?;
    let snapshot = BackupDir::parse(snapshot)?;

    let mut client = HttpClient::new(repo.host(), repo.user(), None)?;

    let keyfile = param["keyfile"].as_str().map(PathBuf::from);

    let crypt_config = match keyfile {
        None => None,
        Some(path) => {
            let (key, _created) = load_and_decrtypt_key(&path, &get_encryption_key_password)?;
            let crypt_config = CryptConfig::new(key)?;
            Some(Arc::new(crypt_config))
        }
    };

    let data = file_get_contents(logfile)?;

    let blob = DataBlob::encode(&data, crypt_config.as_ref().map(Arc::as_ref), true)?;

    let raw_data = blob.into_inner();

    let path = format!("api2/json/admin/datastore/{}/upload-backup-log", repo.store());

    let args = json!({
        "backup-type": snapshot.group().backup_type(),
        "backup-id":  snapshot.group().backup_id(),
        "backup-time": snapshot.backup_time().timestamp(),
    });

    let body = hyper::Body::from(raw_data);

    async_main(async move {
        client.upload("application/octet-stream", body, &path, Some(args)).await
    })
}

fn prune(
    mut param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let repo = extract_repository_from_value(&param)?;

    let mut client = HttpClient::new(repo.host(), repo.user(), None)?;

    let path = format!("api2/json/admin/datastore/{}/prune", repo.store());

    let group = tools::required_string_param(&param, "group")?;
    let group = BackupGroup::parse(group)?;

    param.as_object_mut().unwrap().remove("repository");
    param.as_object_mut().unwrap().remove("group");

    param["backup-type"] = group.backup_type().into();
    param["backup-id"] = group.backup_id().into();

    let _result = async_main(async move { client.post(&path, Some(param)).await })?;

    record_repository(&repo);

    Ok(Value::Null)
}

fn status(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let repo = extract_repository_from_value(&param)?;

    let output_format = param["output-format"].as_str().unwrap_or("text").to_owned();

    let client = HttpClient::new(repo.host(), repo.user(), None)?;

    let path = format!("api2/json/admin/datastore/{}/status", repo.store());

    let result = async_main(async move { client.get(&path, None).await })?;
    let data = &result["data"];

    record_repository(&repo);

    if output_format == "text" {
        let total = data["total"].as_u64().unwrap();
        let used = data["used"].as_u64().unwrap();
        let avail = data["avail"].as_u64().unwrap();
        let roundup = total/200;

        println!(
            "total: {} used: {} ({} %) available: {}",
            total,
            used,
            ((used+roundup)*100)/total,
            avail,
        );
    } else {
        format_and_print_result(data, &output_format);
    }

    Ok(Value::Null)
}

// like get, but simply ignore errors and return Null instead
async fn try_get(repo: &BackupRepository, url: &str) -> Value {

    let client = match HttpClient::new(repo.host(), repo.user(), None) {
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

fn complete_backup_group(_arg: &str, param: &HashMap<String, String>) -> Vec<String> {
    async_main(async { complete_backup_group_do(param).await })
}

async fn complete_backup_group_do(param: &HashMap<String, String>) -> Vec<String> {

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

fn complete_group_or_snapshot(arg: &str, param: &HashMap<String, String>) -> Vec<String> {
    async_main(async { complete_group_or_snapshot_do(arg, param).await })
}

async fn complete_group_or_snapshot_do(arg: &str, param: &HashMap<String, String>) -> Vec<String> {

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

fn complete_backup_snapshot(_arg: &str, param: &HashMap<String, String>) -> Vec<String> {
    async_main(async { complete_backup_snapshot_do(param).await })
}

async fn complete_backup_snapshot_do(param: &HashMap<String, String>) -> Vec<String> {

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
                let snapshot = BackupDir::new(backup_type, backup_id, backup_time);
                result.push(snapshot.relative_path().to_str().unwrap().to_owned());
            }
        }
    }

    result
}

fn complete_server_file_name(_arg: &str, param: &HashMap<String, String>) -> Vec<String> {
    async_main(async { complete_server_file_name_do(param).await })
}

async fn complete_server_file_name_do(param: &HashMap<String, String>) -> Vec<String> {

    let mut result = vec![];

    let repo = match extract_repository_from_map(param) {
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

fn complete_archive_name(arg: &str, param: &HashMap<String, String>) -> Vec<String> {
    complete_server_file_name(arg, param)
        .iter()
        .map(|v| strip_server_file_expenstion(&v))
        .collect()
}

fn complete_chunk_size(_arg: &str, _param: &HashMap<String, String>) -> Vec<String> {

    let mut result = vec![];

    let mut size = 64;
    loop {
        result.push(size.to_string());
        size *= 2;
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

fn master_pubkey_path() -> Result<PathBuf, Error> {
    let base = BaseDirectories::with_prefix("proxmox-backup")?;

    // usually $HOME/.config/proxmox-backup/master-public.pem
    let path = base.place_config_file("master-public.pem")?;

    Ok(path)
}

fn key_import_master_pubkey(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let path = tools::required_string_param(&param, "path")?;
    let path = PathBuf::from(path);

    let pem_data = file_get_contents(&path)?;

    if let Err(err) = openssl::pkey::PKey::public_key_from_pem(&pem_data) {
        bail!("Unable to decode PEM data - {}", err);
    }

    let target_path = master_pubkey_path()?;

    file_set_contents(&target_path, &pem_data, None)?;

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
    file_set_contents(filename_pub, pub_key.as_slice(), None)?;

    let cipher = openssl::symm::Cipher::aes_256_cbc();
    let priv_key: Vec<u8> = pkey.private_key_to_pem_pkcs8_passphrase(cipher, new_pw.as_bytes())?;

    let filename_priv = "master-private.pem";
    println!("Writing private master key to {}", filename_priv);
    file_set_contents(filename_priv, priv_key.as_slice(), None)?;

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

    let (key, created) = load_and_decrtypt_key(&path, &get_encryption_key_password)?;

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

    CliCommandMap::new()
        .insert("create".to_owned(), key_create_cmd_def.into())
        .insert("create-master-key".to_owned(), key_create_master_key_cmd_def.into())
        .insert("import-master-pubkey".to_owned(), key_import_master_pubkey_cmd_def.into())
        .insert("change-passphrase".to_owned(), key_change_passphrase_cmd_def.into())
}


fn mount(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {
    let verbose = param["verbose"].as_bool().unwrap_or(false);
    if verbose {
        // This will stay in foreground with debug output enabled as None is
        // passed for the RawFd.
        return async_main(mount_do(param, None));
    }

    // Process should be deamonized.
    // Make sure to fork before the async runtime is instantiated to avoid troubles.
    let pipe = pipe()?;
    match fork() {
        Ok(ForkResult::Parent { .. }) => {
            nix::unistd::close(pipe.1).unwrap();
            // Blocks the parent process until we are ready to go in the child
            let _res = nix::unistd::read(pipe.0, &mut [0]).unwrap();
            Ok(Value::Null)
        }
        Ok(ForkResult::Child) => {
            nix::unistd::close(pipe.0).unwrap();
            nix::unistd::setsid().unwrap();
            async_main(mount_do(param, Some(pipe.1)))
        }
        Err(_) => bail!("failed to daemonize process"),
    }
}

async fn mount_do(param: Value, pipe: Option<RawFd>) -> Result<Value, Error> {
    let repo = extract_repository_from_value(&param)?;
    let archive_name = tools::required_string_param(&param, "archive-name")?;
    let target = tools::required_string_param(&param, "target")?;
    let client = HttpClient::new(repo.host(), repo.user(), None)?;

    record_repository(&repo);

    let path = tools::required_string_param(&param, "snapshot")?;
    let (backup_type, backup_id, backup_time) = if path.matches('/').count() == 1 {
        let group = BackupGroup::parse(path)?;

        let path = format!("api2/json/admin/datastore/{}/snapshots", repo.store());
        let result = client.get(&path, Some(json!({
            "backup-type": group.backup_type(),
            "backup-id": group.backup_id(),
        }))).await?;

        let list = result["data"].as_array().unwrap();
        if list.is_empty() {
            bail!("backup group '{}' does not contain any snapshots:", path);
        }

        let epoch = list[0]["backup-time"].as_i64().unwrap();
        let backup_time = Utc.timestamp(epoch, 0);
        (group.backup_type().to_owned(), group.backup_id().to_owned(), backup_time)
    } else {
        let snapshot = BackupDir::parse(path)?;
        (snapshot.group().backup_type().to_owned(), snapshot.group().backup_id().to_owned(), snapshot.backup_time())
    };

    let keyfile = param["keyfile"].as_str().map(PathBuf::from);
    let crypt_config = match keyfile {
        None => None,
        Some(path) => {
            let (key, _) = load_and_decrtypt_key(&path, &get_encryption_key_password)?;
            Some(Arc::new(CryptConfig::new(key)?))
        }
    };

    let server_archive_name = if archive_name.ends_with(".pxar") {
        format!("{}.didx", archive_name)
    } else {
        bail!("Can only mount pxar archives.");
    };

    let client = BackupReader::start(
        client,
        crypt_config.clone(),
        repo.store(),
        &backup_type,
        &backup_id,
        backup_time,
        true,
    ).await?;

    let manifest = client.download_manifest().await?;

    if server_archive_name.ends_with(".didx") {
        let index = client.download_dynamic_index(&manifest, &server_archive_name).await?;
        let most_used = index.find_most_used_chunks(8);
        let chunk_reader = RemoteChunkReader::new(client.clone(), crypt_config, most_used);
        let reader = BufferedDynamicReader::new(index, chunk_reader);
        let decoder =
            pxar::Decoder::<Box<dyn pxar::fuse::ReadSeek>, fn(&Path) -> Result<(), Error>>::new(
                Box::new(reader),
                |_| Ok(()),
            )?;
        let options = OsStr::new("ro,default_permissions");
        let mut session = pxar::fuse::Session::from_decoder(decoder, &options, pipe.is_none())
            .map_err(|err| format_err!("pxar mount failed: {}", err))?;

        // Mount the session but not call fuse deamonize as this will cause
        // issues with the runtime after the fork
        let deamonize = false;
        session.mount(&Path::new(target), deamonize)?;

        if let Some(pipe) = pipe {
            nix::unistd::chdir(Path::new("/")).unwrap();
            // Finish creation of deamon by redirecting filedescriptors.
            let nullfd = nix::fcntl::open(
                "/dev/null",
                nix::fcntl::OFlag::O_RDWR,
                nix::sys::stat::Mode::empty(),
            ).unwrap();
            nix::unistd::dup2(nullfd, 0).unwrap();
            nix::unistd::dup2(nullfd, 1).unwrap();
            nix::unistd::dup2(nullfd, 2).unwrap();
            if nullfd > 2 {
                nix::unistd::close(nullfd).unwrap();
            }
            // Signal the parent process that we are done with the setup and it can
            // terminate.
            nix::unistd::write(pipe, &[0u8])?;
            nix::unistd::close(pipe).unwrap();
        }

        let multithreaded = true;
        session.run_loop(multithreaded)?;
    } else {
        bail!("unknown archive file extension (expected .pxar)");
    }

    Ok(Value::Null)
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
                .required(
                    "backupspec",
                    ArraySchema::new(
                        "List of backup source specifications ([<label.ext>:<path>] ...)",
                        backup_source_schema,
                    ).min_length(1)
                )
                .optional("repository", REPO_URL_SCHEMA.clone())
                .optional(
                    "include-dev",
                    ArraySchema::new(
                        "Include mountpoints with same st_dev number (see ``man fstat``) as specified files.",
                        StringSchema::new("Path to file.").into()
                    )
                )
                .optional(
                    "keyfile",
                    StringSchema::new("Path to encryption key. All data will be encrypted using this key."))
                .optional(
                    "verbose",
                    BooleanSchema::new("Verbose output.").default(false))
                .optional(
                    "skip-lost-and-found",
                    BooleanSchema::new("Skip lost+found directory").default(false))
                .optional(
                    "backup-type",
                    BACKUP_TYPE_SCHEMA.clone()
                )
                .optional(
                    "backup-id",
                    BACKUP_ID_SCHEMA.clone()
                )
                .optional(
                    "backup-time",
                    BACKUP_TIME_SCHEMA.clone()
                )
                .optional(
                    "chunk-size",
                    IntegerSchema::new("Chunk size in KB. Must be a power of 2.")
                        .minimum(64)
                        .maximum(4096)
                        .default(4096)
                )
        ))
        .arg_param(vec!["backupspec"])
        .completion_cb("repository", complete_repository)
        .completion_cb("backupspec", complete_backup_source)
        .completion_cb("keyfile", tools::complete_file_name)
        .completion_cb("chunk-size", complete_chunk_size);

    let upload_log_cmd_def = CliCommand::new(
        ApiMethod::new(
            upload_log,
            ObjectSchema::new("Upload backup log file.")
                .required("snapshot", StringSchema::new("Snapshot path."))
                .required("logfile", StringSchema::new("The path to the log file you want to upload."))
                .optional("repository", REPO_URL_SCHEMA.clone())
                .optional(
                    "keyfile",
                    StringSchema::new("Path to encryption key. All data will be encrypted using this key."))
        ))
        .arg_param(vec!["snapshot", "logfile"])
        .completion_cb("snapshot", complete_backup_snapshot)
        .completion_cb("logfile", tools::complete_file_name)
        .completion_cb("keyfile", tools::complete_file_name)
        .completion_cb("repository", complete_repository);

    let list_cmd_def = CliCommand::new(
        ApiMethod::new(
            list_backup_groups,
            ObjectSchema::new("List backup groups.")
                .optional("repository", REPO_URL_SCHEMA.clone())
                .optional("output-format", OUTPUT_FORMAT.clone())
        ))
        .completion_cb("repository", complete_repository);

    let snapshots_cmd_def = CliCommand::new(
        ApiMethod::new(
            list_snapshots,
            ObjectSchema::new("List backup snapshots.")
                .optional("group", StringSchema::new("Backup group."))
                .optional("repository", REPO_URL_SCHEMA.clone())
                .optional("output-format", OUTPUT_FORMAT.clone())
        ))
        .arg_param(vec!["group"])
        .completion_cb("group", complete_backup_group)
        .completion_cb("repository", complete_repository);

    let forget_cmd_def = CliCommand::new(
        ApiMethod::new(
            forget_snapshots,
            ObjectSchema::new("Forget (remove) backup snapshots.")
                .required("snapshot", StringSchema::new("Snapshot path."))
                .optional("repository", REPO_URL_SCHEMA.clone())
        ))
        .arg_param(vec!["snapshot"])
        .completion_cb("repository", complete_repository)
        .completion_cb("snapshot", complete_backup_snapshot);

    let garbage_collect_cmd_def = CliCommand::new(
        ApiMethod::new(
            start_garbage_collection,
            ObjectSchema::new("Start garbage collection for a specific repository.")
                .optional("repository", REPO_URL_SCHEMA.clone())
        ))
        .completion_cb("repository", complete_repository);

    let restore_cmd_def = CliCommand::new(
        ApiMethod::new(
            restore,
            ObjectSchema::new("Restore backup repository.")
                .required("snapshot", StringSchema::new("Group/Snapshot path."))
                .required("archive-name", StringSchema::new("Backup archive name."))
                .required("target", StringSchema::new(r###"Target directory path. Use '-' to write to stdandard output.

We do not extraxt '.pxar' archives when writing to stdandard output.

"###
                ))
                .optional(
                    "allow-existing-dirs",
                    BooleanSchema::new("Do not fail if directories already exists.").default(false))
                .optional("repository", REPO_URL_SCHEMA.clone())
                .optional("keyfile", StringSchema::new("Path to encryption key."))
                .optional(
                    "verbose",
                    BooleanSchema::new("Verbose output.").default(false)
                )
        ))
        .arg_param(vec!["snapshot", "archive-name", "target"])
        .completion_cb("repository", complete_repository)
        .completion_cb("snapshot", complete_group_or_snapshot)
        .completion_cb("archive-name", complete_archive_name)
        .completion_cb("target", tools::complete_file_name);

    let files_cmd_def = CliCommand::new(
        ApiMethod::new(
            list_snapshot_files,
            ObjectSchema::new("List snapshot files.")
                .required("snapshot", StringSchema::new("Snapshot path."))
                .optional("repository", REPO_URL_SCHEMA.clone())
                .optional("output-format", OUTPUT_FORMAT.clone())
        ))
        .arg_param(vec!["snapshot"])
        .completion_cb("repository", complete_repository)
        .completion_cb("snapshot", complete_backup_snapshot);

    let catalog_cmd_def = CliCommand::new(
        ApiMethod::new(
            dump_catalog,
            ObjectSchema::new("Dump catalog.")
                .required("snapshot", StringSchema::new("Snapshot path."))
                .optional("repository", REPO_URL_SCHEMA.clone())
        ))
        .arg_param(vec!["snapshot"])
        .completion_cb("repository", complete_repository)
        .completion_cb("snapshot", complete_backup_snapshot);

    let prune_cmd_def = CliCommand::new(
        ApiMethod::new(
            prune,
            proxmox_backup::api2::admin::datastore::add_common_prune_prameters(
                ObjectSchema::new("Prune backup repository.")
                    .required("group", StringSchema::new("Backup group."))
                    .optional("repository", REPO_URL_SCHEMA.clone())
            )
        ))
        .arg_param(vec!["group"])
        .completion_cb("group", complete_backup_group)
        .completion_cb("repository", complete_repository);

    let status_cmd_def = CliCommand::new(
        ApiMethod::new(
            status,
            ObjectSchema::new("Get repository status.")
                .optional("repository", REPO_URL_SCHEMA.clone())
                .optional("output-format", OUTPUT_FORMAT.clone())
        ))
        .completion_cb("repository", complete_repository);

    let login_cmd_def = CliCommand::new(
        ApiMethod::new(
            api_login,
            ObjectSchema::new("Try to login. If successful, store ticket.")
                .optional("repository", REPO_URL_SCHEMA.clone())
        ))
        .completion_cb("repository", complete_repository);

    let logout_cmd_def = CliCommand::new(
        ApiMethod::new(
            api_logout,
            ObjectSchema::new("Logout (delete stored ticket).")
                .optional("repository", REPO_URL_SCHEMA.clone())
        ))
        .completion_cb("repository", complete_repository);

    let mount_cmd_def = CliCommand::new(
        ApiMethod::new(
            mount,
            ObjectSchema::new("Mount pxar archive.")
                .required("snapshot", StringSchema::new("Group/Snapshot path."))
                .required("archive-name", StringSchema::new("Backup archive name."))
                .required("target", StringSchema::new("Target directory path."))
                .optional("repository", REPO_URL_SCHEMA.clone())
                .optional("keyfile", StringSchema::new("Path to encryption key."))
                .optional("verbose", BooleanSchema::new("Verbose output.").default(false))
        ))
        .arg_param(vec!["snapshot", "archive-name", "target"])
        .completion_cb("repository", complete_repository)
        .completion_cb("snapshot", complete_group_or_snapshot)
        .completion_cb("archive-name", complete_archive_name)
        .completion_cb("target", tools::complete_file_name);

    let cmd_def = CliCommandMap::new()
        .insert("backup".to_owned(), backup_cmd_def.into())
        .insert("upload-log".to_owned(), upload_log_cmd_def.into())
        .insert("forget".to_owned(), forget_cmd_def.into())
        .insert("catalog".to_owned(), catalog_cmd_def.into())
        .insert("garbage-collect".to_owned(), garbage_collect_cmd_def.into())
        .insert("list".to_owned(), list_cmd_def.into())
        .insert("login".to_owned(), login_cmd_def.into())
        .insert("logout".to_owned(), logout_cmd_def.into())
        .insert("prune".to_owned(), prune_cmd_def.into())
        .insert("restore".to_owned(), restore_cmd_def.into())
        .insert("snapshots".to_owned(), snapshots_cmd_def.into())
        .insert("files".to_owned(), files_cmd_def.into())
        .insert("status".to_owned(), status_cmd_def.into())
        .insert("key".to_owned(), key_mgmt_cli().into())
        .insert("mount".to_owned(), mount_cmd_def.into());

    run_cli_command(cmd_def.into());
}

fn async_main<F: Future>(fut: F) -> <F as Future>::Output {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let ret = rt.block_on(fut);
    rt.shutdown_now();
    ret
}
