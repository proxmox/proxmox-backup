//#[macro_use]
extern crate proxmox_backup;

use failure::*;
//use std::os::unix::io::AsRawFd;
use chrono::{Local, Utc, TimeZone};
use std::path::{Path, PathBuf};
use std::collections::{HashSet, HashMap};
use std::io::Write;

use proxmox_backup::tools;
use proxmox_backup::cli::*;
use proxmox_backup::api2::types::*;
use proxmox_backup::api_schema::*;
use proxmox_backup::api_schema::router::*;
use proxmox_backup::client::*;
use proxmox_backup::backup::*;
use proxmox_backup::pxar;

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
    device_set: Option<HashSet<u64>>,
    verbose: bool,
    skip_lost_and_found: bool,
    crypt_config: Option<Arc<CryptConfig>>,
) -> Result<(), Error> {

    let pxar_stream = PxarBackupStream::open(dir_path.as_ref(), device_set, verbose, skip_lost_and_found)?;
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

fn strip_server_file_expenstions(list: Vec<String>) -> Vec<String> {

    let mut result = vec![];

    for file in list.into_iter() {
        if file.ends_with(".didx") {
            result.push(file[..file.len()-5].to_owned());
        } else if file.ends_with(".fidx") {
            result.push(file[..file.len()-5].to_owned());
        } else if file.ends_with(".blob") {
            result.push(file[..file.len()-5].to_owned());
        } else {
            result.push(file); // should not happen
        }
    }

    result
}

fn list_backup_groups(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let repo = extract_repository_from_value(&param)?;

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

        let files = item["files"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_owned()).collect();
        let files = strip_server_file_expenstions(files);

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

    let client = HttpClient::new(repo.host(), repo.user())?;

    let path = format!("api2/json/admin/datastore/{}/snapshots", repo.store());

    let mut args = json!({});
    if let Some(path) = param["group"].as_str() {
        let group = BackupGroup::parse(path)?;
        args["backup-type"] = group.backup_type().into();
        args["backup-id"] = group.backup_id().into();
    }

    let result = client.get(&path, Some(args)).wait()?;

    record_repository(&repo);

    let list = result["data"].as_array().unwrap();

    let mut result = vec![];

    for item in list {

        let id = item["backup-id"].as_str().unwrap();
        let btype = item["backup-type"].as_str().unwrap();
        let epoch = item["backup-time"].as_i64().unwrap();

        let snapshot = BackupDir::new(btype, id, epoch);

        let path = snapshot.relative_path().to_str().unwrap().to_owned();

        let files = item["files"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_owned()).collect();
        let files = strip_server_file_expenstions(files);

        if output_format == "text" {
            println!("{} | {}", path, tools::join(&files, ' '));
        } else {
            result.push(json!({
                "backup-type": btype,
                "backup-id": id,
                "backup-time": epoch,
                "files": files,
            }));
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

    let repo = extract_repository_from_value(&param)?;

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

    let keyfile = param["keyfile"].as_str().map(|p| PathBuf::from(p));

    let backup_id = param["backup-id"].as_str().unwrap_or(&tools::nodename());

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
            "log" => {
                if !file_type.is_file() {
                    bail!("got unexpected file type (expected regular file)");
                }
                upload_list.push((BackupType::LOGFILE, filename.to_owned(), target.to_owned(), metadata.len()));
            }
            _ => {
                bail!("got unknown archive extension '{}'", extension);
            }
        }
    }

    let backup_time = Utc.timestamp(backup_time_opt.unwrap_or(Utc::now().timestamp()), 0);

    let client = HttpClient::new(repo.host(), repo.user())?;
    record_repository(&repo);

    println!("Starting backup: {}/{}/{}", backup_type, backup_id, BackupDir::backup_time_to_string(backup_time));

    println!("Client name: {}", tools::nodename());

    let start_time = Local::now();

    println!("Starting protocol: {}", start_time.to_rfc3339_opts(chrono::SecondsFormat::Secs, false));

    let (crypt_config, rsa_encrypted_key) = match keyfile {
        None => (None, None),
        Some(path) => {
            let (key, created) = load_and_decrtypt_key(&path, get_encryption_key_password)?;

            let crypt_config = CryptConfig::new(key)?;

            let path = master_pubkey_path()?;
            if path.exists() {
                let pem_data = proxmox_backup::tools::file_get_contents(&path)?;
                let rsa = openssl::rsa::Rsa::public_key_from_pem(&pem_data)?;
                let enc_key = crypt_config.generate_rsa_encoded_key(rsa, created)?;
                (Some(Arc::new(crypt_config)), Some(enc_key))
            } else {
                (Some(Arc::new(crypt_config)), None)
            }
        }
    };

    let client = client.start_backup(repo.store(), backup_type, &backup_id, backup_time, verbose).wait()?;

    for (backup_type, filename, target, size) in upload_list {
        match backup_type {
            BackupType::CONFIG => {
                println!("Upload config file '{}' to '{:?}' as {}", filename, repo, target);
                client.upload_blob_from_file(&filename, &target, crypt_config.clone(), true).wait()?;
            }
            BackupType::LOGFILE => { // fixme: remove - not needed anymore ?
                println!("Upload log file '{}' to '{:?}' as {}", filename, repo, target);
                client.upload_blob_from_file(&filename, &target, crypt_config.clone(), true).wait()?;
            }
            BackupType::PXAR => {
                println!("Upload directory '{}' to '{:?}' as {}", filename, repo, target);
                backup_directory(
                    &client,
                    &filename,
                    &target,
                    chunk_size_opt,
                    devices.clone(),
                    verbose,
                    skip_lost_and_found,
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

    if let Some(rsa_encrypted_key) = rsa_encrypted_key {
        let target = "rsa-encrypted.key";
        println!("Upload RSA encoded key to '{:?}' as {}", repo, target);
        client.upload_blob_from_data(rsa_encrypted_key, target, None, false).wait()?;

        // openssl rsautl -decrypt -inkey master-private.pem -in rsa-encrypted.key -out t
        /*
        let mut buffer2 = vec![0u8; rsa.size() as usize];
        let pem_data = proxmox_backup::tools::file_get_contents("master-private.pem")?;
        let rsa = openssl::rsa::Rsa::private_key_from_pem(&pem_data)?;
        let len = rsa.private_decrypt(&buffer, &mut buffer2, openssl::rsa::Padding::PKCS1)?;
        println!("TEST {} {:?}", len, buffer2);
         */
    }

    client.finish().wait()?;

    let end_time = Local::now();
    let elapsed = end_time.signed_duration_since(start_time);
    println!("Duration: {}", elapsed);

    println!("End Time: {}", end_time.to_rfc3339_opts(chrono::SecondsFormat::Secs, false));

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

    let repo = extract_repository_from_value(&param)?;

    let verbose = param["verbose"].as_bool().unwrap_or(false);

    let archive_name = tools::required_string_param(&param, "archive-name")?;

    let client = HttpClient::new(repo.host(), repo.user())?;

    record_repository(&repo);

    let path = tools::required_string_param(&param, "snapshot")?;

    let (backup_type, backup_id, backup_time) = if path.matches('/').count() == 1 {
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

        let epoch = list[0]["backup-time"].as_i64().unwrap();
        let backup_time = Utc.timestamp(epoch, 0);
        (group.backup_type().to_owned(), group.backup_id().to_owned(), backup_time)
    } else {
        let snapshot = BackupDir::parse(path)?;
        (snapshot.group().backup_type().to_owned(), snapshot.group().backup_id().to_owned(), snapshot.backup_time())
    };

    let target = tools::required_string_param(&param, "target")?;
    let target = if target == "-" { None } else { Some(target) };

    let keyfile = param["keyfile"].as_str().map(|p| PathBuf::from(p));

    let crypt_config = match keyfile {
        None => None,
        Some(path) => {
            let (key, _) = load_and_decrtypt_key(&path, get_encryption_key_password)?;
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

    let client = client.start_backup_reader(repo.store(), &backup_type, &backup_id, backup_time, true).wait()?;

    use std::os::unix::fs::OpenOptionsExt;

    let tmpfile = std::fs::OpenOptions::new()
        .write(true)
        .read(true)
        .custom_flags(libc::O_TMPFILE)
        .open("/tmp")?;

    if server_archive_name.ends_with(".blob") {

        let writer = Vec::with_capacity(1024*1024);
        let blob_data = client.download(&server_archive_name, writer).wait()?;
        let blob = DataBlob::from_raw(blob_data)?;
        blob.verify_crc()?;

        let raw_data = match crypt_config {
            Some(ref crypt_config) => blob.decode(Some(crypt_config))?,
            None => blob.decode(None)?,
        };

        if let Some(target) = target {
            crate::tools::file_set_contents(target, &raw_data, None)?;
        } else {
            let stdout = std::io::stdout();
            let mut writer = stdout.lock();
            writer.write_all(&raw_data)
                .map_err(|err| format_err!("unable to pipe data - {}", err))?;
        }

    } else if server_archive_name.ends_with(".didx") {
        let tmpfile = client.download(&server_archive_name, tmpfile).wait()?;

        let index = DynamicIndexReader::new(tmpfile)
            .map_err(|err| format_err!("unable to read dynamic index '{}' - {}", archive_name, err))?;

        let most_used = index.find_most_used_chunks(8);

        let chunk_reader = RemoteChunkReader::new(client.clone(), crypt_config, most_used);

        let mut reader = BufferedDynamicReader::new(index, chunk_reader);

        if let Some(target) = target {

            let feature_flags = pxar::CA_FORMAT_DEFAULT;
            let mut decoder = pxar::SequentialDecoder::new(&mut reader, feature_flags, |path| {
                if verbose {
                    println!("{:?}", path);
                }
                Ok(())
            });

            decoder.restore(Path::new(target), &Vec::new())?;
        } else {
            let stdout = std::io::stdout();
            let mut writer = stdout.lock();

            std::io::copy(&mut reader, &mut writer)
                .map_err(|err| format_err!("unable to pipe data - {}", err))?;
        }
    } else if server_archive_name.ends_with(".fidx") {
        let tmpfile = client.download(&server_archive_name, tmpfile).wait()?;

        let index = FixedIndexReader::new(tmpfile)
            .map_err(|err| format_err!("unable to read fixed index '{}' - {}", archive_name, err))?;

        let most_used = index.find_most_used_chunks(8);

        let chunk_reader = RemoteChunkReader::new(client.clone(), crypt_config, most_used);

        let mut reader = BufferedFixedReader::new(index, chunk_reader);

        if let Some(target) = target {
            let mut writer = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .create_new(true)
                .open(target)
                .map_err(|err| format_err!("unable to create target file {:?} - {}", target, err))?;

            std::io::copy(&mut reader, &mut writer)
                .map_err(|err| format_err!("unable to store data - {}", err))?;
        } else {
            let stdout = std::io::stdout();
            let mut writer = stdout.lock();

            std::io::copy(&mut reader, &mut writer)
                .map_err(|err| format_err!("unable to pipe data - {}", err))?;
        }
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

    let mut client = HttpClient::new(repo.host(), repo.user())?;

    let keyfile = param["keyfile"].as_str().map(|p| PathBuf::from(p));

    let crypt_config = match keyfile {
        None => None,
        Some(path) => {
            let (key, _created) = load_and_decrtypt_key(&path, get_encryption_key_password)?;
            let crypt_config = CryptConfig::new(key)?;
            Some(crypt_config)
        }
    };

    let data = crate::tools::file_get_contents(logfile)?;

    let blob = if let Some(ref crypt_config) = crypt_config {
        DataBlob::encode(&data, Some(crypt_config), true)?
    } else {
        DataBlob::encode(&data, None, true)?
    };

    let raw_data = blob.into_inner();

    let path = format!("api2/json/admin/datastore/{}/upload-backup-log", repo.store());

    let args = json!({
        "backup-type": snapshot.group().backup_type(),
        "backup-id":  snapshot.group().backup_id(),
        "backup-time": snapshot.backup_time().timestamp(),
    });

    let body = hyper::Body::from(raw_data);

    let result = client.upload("application/octet-stream", body, &path, Some(args)).wait()?;

    Ok(result)
}

fn prune(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let repo = extract_repository_from_value(&param)?;

    let mut client = HttpClient::new(repo.host(), repo.user())?;

    let path = format!("api2/json/admin/datastore/{}/prune", repo.store());

    let group = tools::required_string_param(&param, "group")?;
    let group = BackupGroup::parse(group)?;

    let mut args = json!({});
    args["backup-type"] = group.backup_type().into();
    args["backup-id"] = group.backup_id().into();

    let result = client.post(&path, Some(args)).wait()?;

    record_repository(&repo);

    Ok(result)
}

fn status(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let repo = extract_repository_from_value(&param)?;

    let output_format = param["output-format"].as_str().unwrap_or("text").to_owned();

    let client = HttpClient::new(repo.host(), repo.user())?;

    let path = format!("api2/json/admin/datastore/{}/status", repo.store());

    let result = client.get(&path, None).wait()?;
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

fn complete_backup_group(_arg: &str, param: &HashMap<String, String>) -> Vec<String> {

    let mut result = vec![];

    let repo = match extract_repository_from_map(param) {
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

     let repo = match extract_repository_from_map(param) {
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

fn complete_server_file_name(_arg: &str, param: &HashMap<String, String>) -> Vec<String> {

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

    let data = try_get(&repo, &path);

    if let Some(list) = data.as_array() {
        for item in list {
            if let Some(filename) = item.as_str() {
                result.push(filename.to_owned());
            }
        }
    }

    result
}

fn complete_archive_name(arg: &str, param: &HashMap<String, String>) -> Vec<String> {

    let result = complete_server_file_name(arg, param);

    strip_server_file_expenstions(result)
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

    let pem_data = proxmox_backup::tools::file_get_contents(&path)?;

    if let Err(err) = openssl::pkey::PKey::public_key_from_pem(&pem_data) {
        bail!("Unable to decode PEM data - {}", err);
    }

    let target_path = master_pubkey_path()?;

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
        .completion_cb("snapshot", complete_group_or_snapshot)
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
        .completion_cb("snapshot", complete_group_or_snapshot);

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

    let cmd_def = CliCommandMap::new()
        .insert("backup".to_owned(), backup_cmd_def.into())
        .insert("upload-log".to_owned(), upload_log_cmd_def.into())
        .insert("forget".to_owned(), forget_cmd_def.into())
        .insert("garbage-collect".to_owned(), garbage_collect_cmd_def.into())
        .insert("list".to_owned(), list_cmd_def.into())
        .insert("prune".to_owned(), prune_cmd_def.into())
        .insert("restore".to_owned(), restore_cmd_def.into())
        .insert("snapshots".to_owned(), snapshots_cmd_def.into())
        .insert("status".to_owned(), status_cmd_def.into())
        .insert("key".to_owned(), key_mgmt_cli().into());

    hyper::rt::run(futures::future::lazy(move || {
        run_cli_command(cmd_def.into());
        Ok(())
    }));

}
