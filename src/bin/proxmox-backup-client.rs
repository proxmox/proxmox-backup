use failure::*;
use nix::unistd::{fork, ForkResult, pipe};
use std::os::unix::io::RawFd;
use chrono::{Local, DateTime, Utc, TimeZone};
use std::path::{Path, PathBuf};
use std::collections::{HashSet, HashMap};
use std::ffi::OsStr;
use std::io::{Write, Seek, SeekFrom};
use std::os::unix::fs::OpenOptionsExt;

use proxmox::{sortable, identity};
use proxmox::tools::fs::{file_get_contents, file_get_json, replace_file, CreateOptions, image_size};
use proxmox::sys::linux::tty;
use proxmox::api::{ApiHandler, ApiMethod, RpcEnvironment};
use proxmox::api::schema::*;
use proxmox::api::cli::*;
use proxmox::api::api;

use proxmox_backup::tools;
use proxmox_backup::api2::types::*;
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
//use regex::Regex;
use xdg::BaseDirectories;

use futures::*;
use tokio::sync::mpsc;

proxmox::const_regex! {
    BACKUPSPEC_REGEX = r"^([a-zA-Z0-9_-]+\.(?:pxar|img|conf|log)):(.+)$";
}

const REPO_URL_SCHEMA: Schema = StringSchema::new("Repository URL.")
    .format(&BACKUP_REPO_URL)
    .max_length(256)
    .schema();

const BACKUP_SOURCE_SCHEMA: Schema = StringSchema::new(
    "Backup source specification ([<label>:<path>]).")
    .format(&ApiStringFormat::Pattern(&BACKUPSPEC_REGEX))
    .schema();

const KEYFILE_SCHEMA: Schema = StringSchema::new(
    "Path to encryption key. All data will be encrypted using this key.")
    .schema();

const CHUNK_SIZE_SCHEMA: Schema = IntegerSchema::new(
    "Chunk size in KB. Must be a power of 2.")
    .minimum(64)
    .maximum(4096)
    .default(4096)
    .schema();

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

    let _ = replace_file(path, new_data.to_string().as_bytes(), CreateOptions::new());
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

fn connect(server: &str, userid: &str) -> Result<HttpClient, Error> {

    let options = HttpClientOptions::new()
        .prefix(Some("proxmox-backup".to_string()))
        .password_env(Some("PBS_PASSWORD".to_string()))
        .interactive(true)
        .fingerprint_cache(true)
        .ticket_cache(true);

    HttpClient::new(server, userid, options)
}

async fn view_task_result(
    client: HttpClient,
    result: Value,
    output_format: &str,
) -> Result<(), Error> {
    let data = &result["data"];
    if output_format == "text" {
        if let Some(upid) = data.as_str() {
            display_task_log(client, upid, true).await?;
        }
    } else {
        format_and_print_result(&data, &output_format);
    }

    Ok(())
}

async fn api_datastore_list_snapshots(
    client: &HttpClient,
    store: &str,
    group: Option<BackupGroup>,
) -> Result<Vec<SnapshotListItem>, Error> {

    let path = format!("api2/json/admin/datastore/{}/snapshots", store);

    let mut args = json!({});
    if let Some(group) = group {
        args["backup-type"] = group.backup_type().into();
        args["backup-id"] = group.backup_id().into();
    }

    let mut result = client.get(&path, Some(args)).await?;

    let list: Vec<SnapshotListItem> = serde_json::from_value(result["data"].take())?;

    Ok(list)
}

async fn api_datastore_latest_snapshot(
    client: &HttpClient,
    store: &str,
    group: BackupGroup,
) -> Result<(String, String, DateTime<Utc>), Error> {

    let mut list = api_datastore_list_snapshots(client, store, Some(group.clone())).await?;

    if list.is_empty() {
        bail!("backup group {:?} does not contain any snapshots.", group.group_path());
    }

    list.sort_unstable_by(|a, b| b.backup_time.cmp(&a.backup_time));

    let backup_time = Utc.timestamp(list[0].backup_time, 0);

    Ok((group.backup_type().to_owned(), group.backup_id().to_owned(), backup_time))
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
    catalog: Arc<Mutex<CatalogWriter<crate::tools::StdChannelWriter>>>,
    entries_max: usize,
) -> Result<BackupStats, Error> {

    let pxar_stream = PxarBackupStream::open(
        dir_path.as_ref(),
        device_set,
        verbose,
        skip_lost_and_found,
        catalog,
        entries_max,
    )?;
    let mut chunk_stream = ChunkStream::new(pxar_stream, chunk_size);

    let (mut tx, rx) = mpsc::channel(10); // allow to buffer 10 chunks

    let stream = rx
        .map_err(Error::from);

    // spawn chunker inside a separate task so that it can run parallel
    tokio::spawn(async move {
        while let Some(v) = chunk_stream.next().await {
            let _ = tx.send(v).await;
        }
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

    let stream = tokio_util::codec::FramedRead::new(file, tokio_util::codec::BytesCodec::new())
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

#[api(
   input: {
        properties: {
            repository: {
                schema: REPO_URL_SCHEMA,
                optional: true,
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        }
   }
)]
/// List backup groups.
async fn list_backup_groups(param: Value) -> Result<Value, Error> {

    let repo = extract_repository_from_value(&param)?;

    let client = connect(repo.host(), repo.user())?;

    let path = format!("api2/json/admin/datastore/{}/groups", repo.store());

    let mut result = client.get(&path, None).await?;

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

#[api(
   input: {
        properties: {
            repository: {
                schema: REPO_URL_SCHEMA,
                optional: true,
            },
            group: {
                type: String,
                description: "Backup group.",
                optional: true,
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        }
   }
)]
/// List backup snapshots.
async fn list_snapshots(param: Value) -> Result<Value, Error> {

    let repo = extract_repository_from_value(&param)?;

    let output_format = param["output-format"].as_str().unwrap_or("text").to_owned();

    let client = connect(repo.host(), repo.user())?;

    let group = if let Some(path) = param["group"].as_str() {
        Some(BackupGroup::parse(path)?)
    } else {
        None
    };

    let mut list = api_datastore_list_snapshots(&client, repo.store(), group).await?;

    list.sort_unstable_by(|a, b| a.backup_time.cmp(&b.backup_time));

    record_repository(&repo);

    if output_format != "text" {
        format_and_print_result(&serde_json::to_value(list)?, &output_format);
        return Ok(Value::Null);
    }

    for item in list {

        let snapshot = BackupDir::new(item.backup_type, item.backup_id, item.backup_time);

        let path = snapshot.relative_path().to_str().unwrap().to_owned();

        let files = item.files.iter()
            .map(|v| strip_server_file_expenstion(&v))
            .collect();

        let size_str = if let Some(size) = item.size {
            size.to_string()
        } else {
            String::from("-")
        };
        println!("{} | {} | {}", path, size_str, tools::join(&files, ' '));
    }

    Ok(Value::Null)
}

#[api(
   input: {
        properties: {
            repository: {
                schema: REPO_URL_SCHEMA,
                optional: true,
            },
            snapshot: {
                type: String,
                description: "Snapshot path.",
             },
        }
   }
)]
/// Forget (remove) backup snapshots.
async fn forget_snapshots(param: Value) -> Result<Value, Error> {

    let repo = extract_repository_from_value(&param)?;

    let path = tools::required_string_param(&param, "snapshot")?;
    let snapshot = BackupDir::parse(path)?;

    let mut client = connect(repo.host(), repo.user())?;

    let path = format!("api2/json/admin/datastore/{}/snapshots", repo.store());

    let result = client.delete(&path, Some(json!({
        "backup-type": snapshot.group().backup_type(),
        "backup-id": snapshot.group().backup_id(),
        "backup-time": snapshot.backup_time().timestamp(),
    }))).await?;

    record_repository(&repo);

    Ok(result)
}

#[api(
   input: {
        properties: {
            repository: {
                schema: REPO_URL_SCHEMA,
                optional: true,
            },
        }
   }
)]
/// Try to login. If successful, store ticket.
async fn api_login(param: Value) -> Result<Value, Error> {

    let repo = extract_repository_from_value(&param)?;

    let client = connect(repo.host(), repo.user())?;
    client.login().await?;

    record_repository(&repo);

    Ok(Value::Null)
}

#[api(
   input: {
        properties: {
            repository: {
                schema: REPO_URL_SCHEMA,
                optional: true,
            },
        }
   }
)]
/// Logout (delete stored ticket).
fn api_logout(param: Value) -> Result<Value, Error> {

    let repo = extract_repository_from_value(&param)?;

    delete_ticket_info("proxmox-backup", repo.host(), repo.user())?;

    Ok(Value::Null)
}

#[api(
   input: {
        properties: {
            repository: {
                schema: REPO_URL_SCHEMA,
                optional: true,
            },
            snapshot: {
                type: String,
                description: "Snapshot path.",
             },
        }
   }
)]
/// Dump catalog.
async fn dump_catalog(param: Value) -> Result<Value, Error> {

    let repo = extract_repository_from_value(&param)?;

    let path = tools::required_string_param(&param, "snapshot")?;
    let snapshot = BackupDir::parse(path)?;

    let keyfile = param["keyfile"].as_str().map(PathBuf::from);

    let crypt_config = match keyfile {
        None => None,
        Some(path) => {
            let (key, _) = load_and_decrypt_key(&path, &get_encryption_key_password)?;
            Some(Arc::new(CryptConfig::new(key)?))
        }
    };

    let client = connect(repo.host(), repo.user())?;

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

    Ok(Value::Null)
}

#[api(
   input: {
        properties: {
            repository: {
                schema: REPO_URL_SCHEMA,
                optional: true,
            },
            snapshot: {
                type: String,
                description: "Snapshot path.",
             },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        }
   }
)]
/// List snapshot files.
async fn list_snapshot_files(param: Value) -> Result<Value, Error> {

    let repo = extract_repository_from_value(&param)?;

    let path = tools::required_string_param(&param, "snapshot")?;
    let snapshot = BackupDir::parse(path)?;

    let output_format = param["output-format"].as_str().unwrap_or("text").to_owned();

    let client = connect(repo.host(), repo.user())?;

    let path = format!("api2/json/admin/datastore/{}/files", repo.store());

    let mut result = client.get(&path, Some(json!({
        "backup-type": snapshot.group().backup_type(),
        "backup-id": snapshot.group().backup_id(),
        "backup-time": snapshot.backup_time().timestamp(),
    }))).await?;

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

#[api(
    input: {
        properties: {
            repository: {
                schema: REPO_URL_SCHEMA,
                optional: true,
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        },
    },
)]
/// Start garbage collection for a specific repository.
async fn start_garbage_collection(param: Value) -> Result<Value, Error> {

    let repo = extract_repository_from_value(&param)?;
    let output_format = param["output-format"].as_str().unwrap_or("text").to_owned();

    let mut client = connect(repo.host(), repo.user())?;

    let path = format!("api2/json/admin/datastore/{}/gc", repo.store());

    let result = client.post(&path, None).await?;

    record_repository(&repo);

    view_task_result(client, result, &output_format).await?;

    Ok(Value::Null)
}

fn parse_backupspec(value: &str) -> Result<(&str, &str), Error> {

    if let Some(caps) = (BACKUPSPEC_REGEX.regex_obj)().captures(value) {
        return Ok((caps.get(1).unwrap().as_str(), caps.get(2).unwrap().as_str()));
    }
    bail!("unable to parse directory specification '{}'", value);
}

fn spawn_catalog_upload(
    client: Arc<BackupWriter>,
    crypt_config: Option<Arc<CryptConfig>>,
) -> Result<
        (
            Arc<Mutex<CatalogWriter<crate::tools::StdChannelWriter>>>,
            tokio::sync::oneshot::Receiver<Result<BackupStats, Error>>
        ), Error>
{
    let (catalog_tx, catalog_rx) = std::sync::mpsc::sync_channel(10); // allow to buffer 10 writes
    let catalog_stream = crate::tools::StdChannelStream(catalog_rx);
    let catalog_chunk_size = 512*1024;
    let catalog_chunk_stream = ChunkStream::new(catalog_stream, Some(catalog_chunk_size));

    let catalog = Arc::new(Mutex::new(CatalogWriter::new(crate::tools::StdChannelWriter::new(catalog_tx))?));

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

#[api(
   input: {
       properties: {
           backupspec: {
               type: Array,
               description: "List of backup source specifications ([<label.ext>:<path>] ...)",
               items: {
                   schema: BACKUP_SOURCE_SCHEMA,
               }
           },
           repository: {
               schema: REPO_URL_SCHEMA,
               optional: true,
           },
           "include-dev": {
               description: "Include mountpoints with same st_dev number (see ``man fstat``) as specified files.",
               optional: true,
               items: {
                   type: String,
                   description: "Path to file.",
               }
           },
           keyfile: {
               schema: KEYFILE_SCHEMA,
               optional: true,
           },
           "skip-lost-and-found": {
               type: Boolean,
               description: "Skip lost+found directory.",
               optional: true,
           },
           "backup-type": {
               schema: BACKUP_TYPE_SCHEMA,
               optional: true,
           },
           "backup-id": {
               schema: BACKUP_ID_SCHEMA,
               optional: true,
           },
           "backup-time": {
               schema: BACKUP_TIME_SCHEMA,
               optional: true,
           },
           "chunk-size": {
               schema: CHUNK_SIZE_SCHEMA,
               optional: true,
           },
           "entries-max": {
               type: Integer,
               description: "Max number of entries to hold in memory.",
               optional: true,
               default: pxar::ENCODER_MAX_ENTRIES as isize,
           },
       }
   }
)]
/// Create (host) backup.
async fn create_backup(
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

    let entries_max = param["entries-max"].as_u64().unwrap_or(pxar::ENCODER_MAX_ENTRIES as u64);

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

    let client = connect(repo.host(), repo.user())?;
    record_repository(&repo);

    println!("Starting backup: {}/{}/{}", backup_type, backup_id, BackupDir::backup_time_to_string(backup_time));

    println!("Client name: {}", proxmox::tools::nodename());

    let start_time = Local::now();

    println!("Starting protocol: {}", start_time.to_rfc3339_opts(chrono::SecondsFormat::Secs, false));

    let (crypt_config, rsa_encrypted_key) = match keyfile {
        None => (None, None),
        Some(path) => {
            let (key, created) = load_and_decrypt_key(&path, &get_encryption_key_password)?;

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
                manifest.add_file(target, stats.size, stats.csum)?;
            }
            BackupType::LOGFILE => { // fixme: remove - not needed anymore ?
                println!("Upload log file '{}' to '{:?}' as {}", filename, repo, target);
                let stats = client
                    .upload_blob_from_file(&filename, &target, crypt_config.clone(), true)
                    .await?;
                manifest.add_file(target, stats.size, stats.csum)?;
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
                    entries_max as usize,
                ).await?;
                manifest.add_file(target, stats.size, stats.csum)?;
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
                manifest.add_file(target, stats.size, stats.csum)?;
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

        manifest.add_file(CATALOG_NAME.to_owned(), stats.size, stats.csum)?;
    }

    if let Some(rsa_encrypted_key) = rsa_encrypted_key {
        let target = "rsa-encrypted.key";
        println!("Upload RSA encoded key to '{:?}' as {}", repo, target);
        let stats = client
            .upload_blob_from_data(rsa_encrypted_key, target, None, false, false)
            .await?;
        manifest.add_file(format!("{}.blob", target), stats.size, stats.csum)?;

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

#[api(
   input: {
       properties: {
           repository: {
               schema: REPO_URL_SCHEMA,
               optional: true,
           },
           snapshot: {
               type: String,
               description: "Group/Snapshot path.",
           },
           "archive-name": {
               description: "Backup archive name.",
               type: String,
           },
           target: {
               type: String,
               description: r###"Target directory path. Use '-' to write to standard output.

We do not extraxt '.pxar' archives when writing to standard output.

"###
           },
           "allow-existing-dirs": {
               type: Boolean,
               description: "Do not fail if directories already exists.",
               optional: true,
           },
           keyfile: {
               schema: KEYFILE_SCHEMA,
               optional: true,
           },
       }
   }
)]
/// Restore backup repository.
async fn restore(param: Value) -> Result<Value, Error> {
    let repo = extract_repository_from_value(&param)?;

    let verbose = param["verbose"].as_bool().unwrap_or(false);

    let allow_existing_dirs = param["allow-existing-dirs"].as_bool().unwrap_or(false);

    let archive_name = tools::required_string_param(&param, "archive-name")?;

    let client = connect(repo.host(), repo.user())?;

    record_repository(&repo);

    let path = tools::required_string_param(&param, "snapshot")?;

    let (backup_type, backup_id, backup_time) = if path.matches('/').count() == 1 {
        let group = BackupGroup::parse(path)?;
        api_datastore_latest_snapshot(&client, repo.store(), group).await?
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
            let (key, _) = load_and_decrypt_key(&path, &get_encryption_key_password)?;
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

    let manifest = client.download_manifest().await?;

    if server_archive_name == MANIFEST_BLOB_NAME {
        let backup_index_data = manifest.into_json().to_string();
        if let Some(target) = target {
            replace_file(target, backup_index_data.as_bytes(), CreateOptions::new())?;
        } else {
            let stdout = std::io::stdout();
            let mut writer = stdout.lock();
            writer.write_all(backup_index_data.as_bytes())
                .map_err(|err| format_err!("unable to pipe data - {}", err))?;
        }

    } else if server_archive_name.ends_with(".blob") {

        let mut reader = client.download_blob(&manifest, &server_archive_name).await?;

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
            let mut decoder = pxar::SequentialDecoder::new(&mut reader, feature_flags);
            decoder.set_callback(move |path| {
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

        let index = client.download_fixed_index(&manifest, &server_archive_name).await?;

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

#[api(
   input: {
       properties: {
           repository: {
               schema: REPO_URL_SCHEMA,
               optional: true,
           },
           snapshot: {
               type: String,
               description: "Group/Snapshot path.",
           },
           logfile: {
               type: String,
               description: "The path to the log file you want to upload.",
           },
           keyfile: {
               schema: KEYFILE_SCHEMA,
               optional: true,
           },
       }
   }
)]
/// Upload backup log file.
async fn upload_log(param: Value) -> Result<Value, Error> {

    let logfile = tools::required_string_param(&param, "logfile")?;
    let repo = extract_repository_from_value(&param)?;

    let snapshot = tools::required_string_param(&param, "snapshot")?;
    let snapshot = BackupDir::parse(snapshot)?;

    let mut client = connect(repo.host(), repo.user())?;

    let keyfile = param["keyfile"].as_str().map(PathBuf::from);

    let crypt_config = match keyfile {
        None => None,
        Some(path) => {
            let (key, _created) = load_and_decrypt_key(&path, &get_encryption_key_password)?;
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

    client.upload("application/octet-stream", body, &path, Some(args)).await
}

const API_METHOD_PRUNE: ApiMethod = ApiMethod::new(
    &ApiHandler::Async(&prune),
    &ObjectSchema::new(
        "Prune a backup repository.",
        &proxmox_backup::add_common_prune_prameters!([
            ("dry-run", true, &BooleanSchema::new(
                "Just show what prune would do, but do not delete anything.")
             .schema()),
            ("group", false, &StringSchema::new("Backup group.").schema()),
        ], [
            ("output-format", true, &OUTPUT_FORMAT),
            ("repository", true, &REPO_URL_SCHEMA),
        ])
    )
);

fn prune<'a>(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &'a mut dyn RpcEnvironment,
) -> proxmox::api::ApiFuture<'a> {
    async move {
        prune_async(param).await
    }.boxed()
}

async fn prune_async(mut param: Value) -> Result<Value, Error> {
    let repo = extract_repository_from_value(&param)?;

    let mut client = connect(repo.host(), repo.user())?;

    let path = format!("api2/json/admin/datastore/{}/prune", repo.store());

    let group = tools::required_string_param(&param, "group")?;
    let group = BackupGroup::parse(group)?;
    let output_format = param["output-format"].as_str().unwrap_or("text").to_owned();

    param.as_object_mut().unwrap().remove("repository");
    param.as_object_mut().unwrap().remove("group");
    param.as_object_mut().unwrap().remove("output-format");

    param["backup-type"] = group.backup_type().into();
    param["backup-id"] = group.backup_id().into();

    let result = client.post(&path, Some(param)).await?;

    record_repository(&repo);

    view_task_result(client, result, &output_format).await?;

    Ok(Value::Null)
}

#[api(
   input: {
       properties: {
           repository: {
               schema: REPO_URL_SCHEMA,
               optional: true,
           },
           "output-format": {
               schema: OUTPUT_FORMAT,
               optional: true,
           },
       }
   }
)]
/// Get repository status.
async fn status(param: Value) -> Result<Value, Error> {

    let repo = extract_repository_from_value(&param)?;

    let output_format = param["output-format"].as_str().unwrap_or("text").to_owned();

    let client = connect(repo.host(), repo.user())?;

    let path = format!("api2/json/admin/datastore/{}/status", repo.store());

    let mut result = client.get(&path, None).await?;

    record_repository(&repo);

    if output_format == "text" {
        let result: StorageStatus = serde_json::from_value(result["data"].take())?;

        let roundup = result.total/200;

        println!(
            "total: {} used: {} ({} %) available: {}",
            result.total,
            result.used,
            ((result.used+roundup)*100)/result.total,
            result.avail,
        );
    } else {
        format_and_print_result(&result["data"], &output_format);
    }

    Ok(Value::Null)
}

// like get, but simply ignore errors and return Null instead
async fn try_get(repo: &BackupRepository, url: &str) -> Value {

    let options = HttpClientOptions::new()
        .prefix(Some("proxmox-backup".to_string()))
        .password_env(Some("PBS_PASSWORD".to_string()))
        .interactive(false)
        .fingerprint_cache(true)
        .ticket_cache(true);

    let client = match HttpClient::new(repo.host(), repo.user(), options) {
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
    proxmox_backup::tools::runtime::main(async { complete_backup_group_do(param).await })
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
    proxmox_backup::tools::runtime::main(async { complete_group_or_snapshot_do(arg, param).await })
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
    proxmox_backup::tools::runtime::main(async { complete_backup_snapshot_do(param).await })
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
    proxmox_backup::tools::runtime::main(async { complete_server_file_name_do(param).await })
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

fn complete_pxar_archive_name(arg: &str, param: &HashMap<String, String>) -> Vec<String> {
    complete_server_file_name(arg, param)
        .iter()
        .filter_map(|v| {
            let name = strip_server_file_expenstion(&v);
            if name.ends_with(".pxar") {
                Some(name)
            } else {
                None
            }
        })
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
    if tty::stdin_isatty() {
        return Ok(tty::read_password("Encryption Key Password: ")?);
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
        if !tty::stdin_isatty() {
            bail!("unable to read passphrase - no tty");
        }

        let password = tty::read_and_verify_password("Encryption Key Password: ")?;

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

    replace_file(&target_path, &pem_data, CreateOptions::new())?;

    println!("Imported public master key to {:?}", target_path);

    Ok(Value::Null)
}

fn key_create_master_key(
    _param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    // we need a TTY to query the new password
    if !tty::stdin_isatty() {
        bail!("unable to create master key - no tty");
    }

    let rsa = openssl::rsa::Rsa::generate(4096)?;
    let pkey = openssl::pkey::PKey::from_rsa(rsa)?;


    let password = String::from_utf8(tty::read_and_verify_password("Master Key Password: ")?)?;

    let pub_key: Vec<u8> = pkey.public_key_to_pem()?;
    let filename_pub = "master-public.pem";
    println!("Writing public master key to {}", filename_pub);
    replace_file(filename_pub, pub_key.as_slice(), CreateOptions::new())?;

    let cipher = openssl::symm::Cipher::aes_256_cbc();
    let priv_key: Vec<u8> = pkey.private_key_to_pem_pkcs8_passphrase(cipher, password.as_bytes())?;

    let filename_priv = "master-private.pem";
    println!("Writing private master key to {}", filename_priv);
    replace_file(filename_priv, priv_key.as_slice(), CreateOptions::new())?;

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
    if !tty::stdin_isatty() {
        bail!("unable to change passphrase - no tty");
    }

    let (key, created) = load_and_decrypt_key(&path, &get_encryption_key_password)?;

    if kdf == "scrypt" {

        let password = tty::read_and_verify_password("New Password: ")?;

        let mut new_key_config = encrypt_key_with_passphrase(&key, &password)?;
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

    const KDF_SCHEMA: Schema =
        StringSchema::new("Key derivation function. Choose 'none' to store the key unecrypted.")
        .format(&ApiStringFormat::Enum(&["scrypt", "none"]))
        .default("scrypt")
        .schema();

    #[sortable]
    const API_METHOD_KEY_CREATE: ApiMethod = ApiMethod::new(
        &ApiHandler::Sync(&key_create),
        &ObjectSchema::new(
            "Create a new encryption key.",
            &sorted!([
                ("path", false, &StringSchema::new("File system path.").schema()),
                ("kdf", true, &KDF_SCHEMA),
            ]),
        )
    );

    let key_create_cmd_def = CliCommand::new(&API_METHOD_KEY_CREATE)
        .arg_param(&["path"])
        .completion_cb("path", tools::complete_file_name);

    #[sortable]
    const API_METHOD_KEY_CHANGE_PASSPHRASE: ApiMethod = ApiMethod::new(
        &ApiHandler::Sync(&key_change_passphrase),
        &ObjectSchema::new(
            "Change the passphrase required to decrypt the key.",
            &sorted!([
                ("path", false, &StringSchema::new("File system path.").schema()),
                ("kdf", true, &KDF_SCHEMA),
            ]),
        )
    );

    let key_change_passphrase_cmd_def = CliCommand::new(&API_METHOD_KEY_CHANGE_PASSPHRASE)
        .arg_param(&["path"])
        .completion_cb("path", tools::complete_file_name);

    const API_METHOD_KEY_CREATE_MASTER_KEY: ApiMethod = ApiMethod::new(
        &ApiHandler::Sync(&key_create_master_key),
        &ObjectSchema::new("Create a new 4096 bit RSA master pub/priv key pair.", &[])
    );

    let key_create_master_key_cmd_def = CliCommand::new(&API_METHOD_KEY_CREATE_MASTER_KEY);

    #[sortable]
    const API_METHOD_KEY_IMPORT_MASTER_PUBKEY: ApiMethod = ApiMethod::new(
        &ApiHandler::Sync(&key_import_master_pubkey),
        &ObjectSchema::new(
            "Import a new RSA public key and use it as master key. The key is expected to be in '.pem' format.",
            &sorted!([ ("path", false, &StringSchema::new("File system path.").schema()) ]),
        )
    );

    let key_import_master_pubkey_cmd_def = CliCommand::new(&API_METHOD_KEY_IMPORT_MASTER_PUBKEY)
        .arg_param(&["path"])
        .completion_cb("path", tools::complete_file_name);

    CliCommandMap::new()
        .insert("create", key_create_cmd_def)
        .insert("create-master-key", key_create_master_key_cmd_def)
        .insert("import-master-pubkey", key_import_master_pubkey_cmd_def)
        .insert("change-passphrase", key_change_passphrase_cmd_def)
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
        return proxmox_backup::tools::runtime::main(mount_do(param, None));
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
            proxmox_backup::tools::runtime::main(mount_do(param, Some(pipe.1)))
        }
        Err(_) => bail!("failed to daemonize process"),
    }
}

async fn mount_do(param: Value, pipe: Option<RawFd>) -> Result<Value, Error> {
    let repo = extract_repository_from_value(&param)?;
    let archive_name = tools::required_string_param(&param, "archive-name")?;
    let target = tools::required_string_param(&param, "target")?;
    let client = connect(repo.host(), repo.user())?;

    record_repository(&repo);

    let path = tools::required_string_param(&param, "snapshot")?;
    let (backup_type, backup_id, backup_time) = if path.matches('/').count() == 1 {
        let group = BackupGroup::parse(path)?;
        api_datastore_latest_snapshot(&client, repo.store(), group).await?
    } else {
        let snapshot = BackupDir::parse(path)?;
        (snapshot.group().backup_type().to_owned(), snapshot.group().backup_id().to_owned(), snapshot.backup_time())
    };

    let keyfile = param["keyfile"].as_str().map(PathBuf::from);
    let crypt_config = match keyfile {
        None => None,
        Some(path) => {
            let (key, _) = load_and_decrypt_key(&path, &get_encryption_key_password)?;
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
        let decoder = pxar::Decoder::new(reader)?;
        let options = OsStr::new("ro,default_permissions");
        let mut session = pxar::fuse::Session::new(decoder, &options, pipe.is_none())
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

#[api(
    input: {
        properties: {
            "snapshot": {
                type: String,
                description: "Group/Snapshot path.",
            },
            "archive-name": {
                type: String,
                description: "Backup archive name.",
            },
            "repository": {
                optional: true,
                schema: REPO_URL_SCHEMA,
            },
            "keyfile": {
                optional: true,
                type: String,
                description: "Path to encryption key.",
            },
        },
    },
)]
/// Shell to interactively inspect and restore snapshots.
async fn catalog_shell(param: Value) -> Result<(), Error> {
    let repo = extract_repository_from_value(&param)?;
    let client = connect(repo.host(), repo.user())?;
    let path = tools::required_string_param(&param, "snapshot")?;
    let archive_name = tools::required_string_param(&param, "archive-name")?;

    let (backup_type, backup_id, backup_time) = if path.matches('/').count() == 1 {
        let group = BackupGroup::parse(path)?;
        api_datastore_latest_snapshot(&client, repo.store(), group).await?
    } else {
        let snapshot = BackupDir::parse(path)?;
        (snapshot.group().backup_type().to_owned(), snapshot.group().backup_id().to_owned(), snapshot.backup_time())
    };

    let keyfile = param["keyfile"].as_str().map(|p| PathBuf::from(p));
    let crypt_config = match keyfile {
        None => None,
        Some(path) => {
            let (key, _) = load_and_decrypt_key(&path, &get_encryption_key_password)?;
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

    let tmpfile = std::fs::OpenOptions::new()
        .write(true)
        .read(true)
        .custom_flags(libc::O_TMPFILE)
        .open("/tmp")?;

    let manifest = client.download_manifest().await?;

    let index = client.download_dynamic_index(&manifest, &server_archive_name).await?;
    let most_used = index.find_most_used_chunks(8);
    let chunk_reader = RemoteChunkReader::new(client.clone(), crypt_config.clone(), most_used);
    let reader = BufferedDynamicReader::new(index, chunk_reader);
    let mut decoder = pxar::Decoder::new(reader)?;
    decoder.set_callback(|path| {
        println!("{:?}", path);
        Ok(())
    });

    let tmpfile = client.download(CATALOG_NAME, tmpfile).await?;
    let index = DynamicIndexReader::new(tmpfile)
        .map_err(|err| format_err!("unable to read catalog index - {}", err))?;

    // Note: do not use values stored in index (not trusted) - instead, computed them again
    let (csum, size) = index.compute_csum();
    manifest.verify_file(CATALOG_NAME, &csum, size)?;

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
    let catalog_reader = CatalogReader::new(catalogfile);
    let state = Shell::new(
        catalog_reader,
        &server_archive_name,
        decoder,
    )?;

    println!("Starting interactive shell");
    state.shell()?;

    record_repository(&repo);

    Ok(())
}

fn catalog_mgmt_cli() -> CliCommandMap {
    let catalog_shell_cmd_def = CliCommand::new(&API_METHOD_CATALOG_SHELL)
        .arg_param(&["snapshot", "archive-name"])
        .completion_cb("repository", complete_repository)
        .completion_cb("archive-name", complete_pxar_archive_name)
        .completion_cb("snapshot", complete_group_or_snapshot);

    let catalog_dump_cmd_def = CliCommand::new(&API_METHOD_DUMP_CATALOG)
        .arg_param(&["snapshot"])
        .completion_cb("repository", complete_repository)
        .completion_cb("snapshot", complete_backup_snapshot);

    CliCommandMap::new()
        .insert("dump", catalog_dump_cmd_def)
        .insert("shell", catalog_shell_cmd_def)
}

#[api(
    input: {
        properties: {
            repository: {
                schema: REPO_URL_SCHEMA,
                optional: true,
            },
            limit: {
                description: "The maximal number of tasks to list.",
                type: Integer,
                optional: true,
                minimum: 1,
                maximum: 1000,
                default: 50,
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        }
    }
)]
/// List running server tasks for this repo user
async fn task_list(param: Value) -> Result<Value, Error> {

    let output_format = param["output-format"].as_str().unwrap_or("text").to_owned();
    let repo = extract_repository_from_value(&param)?;
    let client = connect(repo.host(), repo.user())?;

    let limit = param["limit"].as_u64().unwrap_or(50) as usize;

    let args = json!({
        "running": true,
        "start": 0,
        "limit": limit,
        "userfilter": repo.user(),
        "store": repo.store(),
    });
    let result = client.get("api2/json/nodes/localhost/tasks", Some(args)).await?;

    let data = &result["data"];

    if output_format == "text" {
        for item in data.as_array().unwrap() {
            println!(
                "{} {}",
                item["upid"].as_str().unwrap(),
                item["status"].as_str().unwrap_or("running"),
            );
        }
    } else {
        format_and_print_result(data, &output_format);
    }

    Ok(Value::Null)
}

#[api(
    input: {
        properties: {
            repository: {
                schema: REPO_URL_SCHEMA,
                optional: true,
            },
            upid: {
                schema: UPID_SCHEMA,
            },
        }
    }
)]
/// Display the task log.
async fn task_log(param: Value) -> Result<Value, Error> {

    let repo = extract_repository_from_value(&param)?;
    let upid =  tools::required_string_param(&param, "upid")?;

    let client = connect(repo.host(), repo.user())?;

    display_task_log(client, upid, true).await?;

    Ok(Value::Null)
}

#[api(
    input: {
        properties: {
            repository: {
                schema: REPO_URL_SCHEMA,
                optional: true,
            },
            upid: {
                schema: UPID_SCHEMA,
            },
        }
    }
)]
/// Try to stop a specific task.
async fn task_stop(param: Value) -> Result<Value, Error> {

    let repo = extract_repository_from_value(&param)?;
    let upid_str =  tools::required_string_param(&param, "upid")?;

    let mut client = connect(repo.host(), repo.user())?;

    let path = format!("api2/json/nodes/localhost/tasks/{}", upid_str);
    let _ = client.delete(&path, None).await?;

    Ok(Value::Null)
}

fn task_mgmt_cli() -> CliCommandMap {

    let task_list_cmd_def = CliCommand::new(&API_METHOD_TASK_LIST)
        .completion_cb("repository", complete_repository);

    let task_log_cmd_def = CliCommand::new(&API_METHOD_TASK_LOG)
        .arg_param(&["upid"]);

    let task_stop_cmd_def = CliCommand::new(&API_METHOD_TASK_STOP)
        .arg_param(&["upid"]);

    CliCommandMap::new()
        .insert("log", task_log_cmd_def)
        .insert("list", task_list_cmd_def)
        .insert("stop", task_stop_cmd_def)
}

fn main() {

    let backup_cmd_def = CliCommand::new(&API_METHOD_CREATE_BACKUP)
        .arg_param(&["backupspec"])
        .completion_cb("repository", complete_repository)
        .completion_cb("backupspec", complete_backup_source)
        .completion_cb("keyfile", tools::complete_file_name)
        .completion_cb("chunk-size", complete_chunk_size);

    let upload_log_cmd_def = CliCommand::new(&API_METHOD_UPLOAD_LOG)
        .arg_param(&["snapshot", "logfile"])
        .completion_cb("snapshot", complete_backup_snapshot)
        .completion_cb("logfile", tools::complete_file_name)
        .completion_cb("keyfile", tools::complete_file_name)
        .completion_cb("repository", complete_repository);

    let list_cmd_def = CliCommand::new(&API_METHOD_LIST_BACKUP_GROUPS)
        .completion_cb("repository", complete_repository);

    let snapshots_cmd_def = CliCommand::new(&API_METHOD_LIST_SNAPSHOTS)
        .arg_param(&["group"])
        .completion_cb("group", complete_backup_group)
        .completion_cb("repository", complete_repository);

    let forget_cmd_def = CliCommand::new(&API_METHOD_FORGET_SNAPSHOTS)
        .arg_param(&["snapshot"])
        .completion_cb("repository", complete_repository)
        .completion_cb("snapshot", complete_backup_snapshot);

    let garbage_collect_cmd_def = CliCommand::new(&API_METHOD_START_GARBAGE_COLLECTION)
        .completion_cb("repository", complete_repository);

    let restore_cmd_def = CliCommand::new(&API_METHOD_RESTORE)
        .arg_param(&["snapshot", "archive-name", "target"])
        .completion_cb("repository", complete_repository)
        .completion_cb("snapshot", complete_group_or_snapshot)
        .completion_cb("archive-name", complete_archive_name)
        .completion_cb("target", tools::complete_file_name);

    let files_cmd_def = CliCommand::new(&API_METHOD_LIST_SNAPSHOT_FILES)
        .arg_param(&["snapshot"])
        .completion_cb("repository", complete_repository)
        .completion_cb("snapshot", complete_backup_snapshot);

    let prune_cmd_def = CliCommand::new(&API_METHOD_PRUNE)
        .arg_param(&["group"])
        .completion_cb("group", complete_backup_group)
        .completion_cb("repository", complete_repository);

    let status_cmd_def = CliCommand::new(&API_METHOD_STATUS)
        .completion_cb("repository", complete_repository);

    let login_cmd_def = CliCommand::new(&API_METHOD_API_LOGIN)
        .completion_cb("repository", complete_repository);

    let logout_cmd_def = CliCommand::new(&API_METHOD_API_LOGOUT)
        .completion_cb("repository", complete_repository);

    #[sortable]
    const API_METHOD_MOUNT: ApiMethod = ApiMethod::new(
        &ApiHandler::Sync(&mount),
        &ObjectSchema::new(
            "Mount pxar archive.",
            &sorted!([
                ("snapshot", false, &StringSchema::new("Group/Snapshot path.").schema()),
                ("archive-name", false, &StringSchema::new("Backup archive name.").schema()),
                ("target", false, &StringSchema::new("Target directory path.").schema()),
                ("repository", true, &REPO_URL_SCHEMA),
                ("keyfile", true, &StringSchema::new("Path to encryption key.").schema()),
                ("verbose", true, &BooleanSchema::new("Verbose output.").default(false).schema()),
            ]),
        )
    );

    let mount_cmd_def = CliCommand::new(&API_METHOD_MOUNT)
        .arg_param(&["snapshot", "archive-name", "target"])
        .completion_cb("repository", complete_repository)
        .completion_cb("snapshot", complete_group_or_snapshot)
        .completion_cb("archive-name", complete_pxar_archive_name)
        .completion_cb("target", tools::complete_file_name);


    let cmd_def = CliCommandMap::new()
        .insert("backup", backup_cmd_def)
        .insert("upload-log", upload_log_cmd_def)
        .insert("forget", forget_cmd_def)
        .insert("garbage-collect", garbage_collect_cmd_def)
        .insert("list", list_cmd_def)
        .insert("login", login_cmd_def)
        .insert("logout", logout_cmd_def)
        .insert("prune", prune_cmd_def)
        .insert("restore", restore_cmd_def)
        .insert("snapshots", snapshots_cmd_def)
        .insert("files", files_cmd_def)
        .insert("status", status_cmd_def)
        .insert("key", key_mgmt_cli())
        .insert("mount", mount_cmd_def)
        .insert("catalog", catalog_mgmt_cli())
        .insert("task", task_mgmt_cli());

    run_cli_command(cmd_def, Some(|future| {
        proxmox_backup::tools::runtime::main(future)
    }));
}
