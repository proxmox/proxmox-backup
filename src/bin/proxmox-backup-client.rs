use std::collections::{HashSet, HashMap};
use std::convert::TryFrom;
use std::io::{self, Read, Write, Seek, SeekFrom};
use std::os::unix::io::{FromRawFd, RawFd};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::Context;

use anyhow::{bail, format_err, Error};
use futures::future::FutureExt;
use futures::stream::{StreamExt, TryStreamExt};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use xdg::BaseDirectories;

use pathpatterns::{MatchEntry, MatchType, PatternFlag};
use proxmox::{
    tools::{
        time::{strftime_local, epoch_i64},
        fs::{file_get_contents, file_get_json, replace_file, CreateOptions, image_size},
    },
    api::{
        api,
        ApiHandler,
        ApiMethod,
        RpcEnvironment,
        schema::*,
        cli::*,
    },
};
use pxar::accessor::{MaybeReady, ReadAt, ReadAtOperation};

use proxmox_backup::tools;
use proxmox_backup::api2::access::user::UserWithTokens;
use proxmox_backup::api2::types::*;
use proxmox_backup::api2::version;
use proxmox_backup::client::*;
use proxmox_backup::pxar::catalog::*;
use proxmox_backup::backup::{
    archive_type,
    decrypt_key,
    rsa_encrypt_key_config,
    verify_chunk_size,
    ArchiveType,
    AsyncReadChunk,
    BackupDir,
    BackupGroup,
    BackupManifest,
    BufferedDynamicReader,
    CATALOG_NAME,
    CatalogReader,
    CatalogWriter,
    ChunkStream,
    CryptConfig,
    CryptMode,
    DynamicIndexReader,
    ENCRYPTED_KEY_BLOB_NAME,
    FixedChunkStream,
    FixedIndexReader,
    KeyConfig,
    IndexFile,
    MANIFEST_BLOB_NAME,
    Shell,
};

mod proxmox_backup_client;
use proxmox_backup_client::*;

const ENV_VAR_PBS_FINGERPRINT: &str = "PBS_FINGERPRINT";
const ENV_VAR_PBS_PASSWORD: &str = "PBS_PASSWORD";


pub const REPO_URL_SCHEMA: Schema = StringSchema::new("Repository URL.")
    .format(&BACKUP_REPO_URL)
    .max_length(256)
    .schema();

pub const KEYFILE_SCHEMA: Schema = StringSchema::new(
    "Path to encryption key. All data will be encrypted using this key.")
    .schema();

pub const KEYFD_SCHEMA: Schema = IntegerSchema::new(
    "Pass an encryption key via an already opened file descriptor.")
    .minimum(0)
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

pub fn extract_repository_from_value(
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

fn connect(repo: &BackupRepository) -> Result<HttpClient, Error> {
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

    let options = HttpClientOptions::new()
        .prefix(Some("proxmox-backup".to_string()))
        .password(password)
        .interactive(true)
        .fingerprint(fingerprint)
        .fingerprint_cache(true)
        .ticket_cache(true);

    HttpClient::new(server, port, auth_id, options)
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
) -> Result<Value, Error> {

    let path = format!("api2/json/admin/datastore/{}/snapshots", store);

    let mut args = json!({});
    if let Some(group) = group {
        args["backup-type"] = group.backup_type().into();
        args["backup-id"] = group.backup_id().into();
    }

    let mut result = client.get(&path, Some(args)).await?;

    Ok(result["data"].take())
}

pub async fn api_datastore_latest_snapshot(
    client: &HttpClient,
    store: &str,
    group: BackupGroup,
) -> Result<(String, String, i64), Error> {

    let list = api_datastore_list_snapshots(client, store, Some(group.clone())).await?;
    let mut list: Vec<SnapshotListItem> = serde_json::from_value(list)?;

    if list.is_empty() {
        bail!("backup group {:?} does not contain any snapshots.", group.group_path());
    }

    list.sort_unstable_by(|a, b| b.backup_time.cmp(&a.backup_time));

    let backup_time = list[0].backup_time;

    Ok((group.backup_type().to_owned(), group.backup_id().to_owned(), backup_time))
}

async fn backup_directory<P: AsRef<Path>>(
    client: &BackupWriter,
    previous_manifest: Option<Arc<BackupManifest>>,
    dir_path: P,
    archive_name: &str,
    chunk_size: Option<usize>,
    device_set: Option<HashSet<u64>>,
    verbose: bool,
    skip_lost_and_found: bool,
    catalog: Arc<Mutex<CatalogWriter<crate::tools::StdChannelWriter>>>,
    exclude_pattern: Vec<MatchEntry>,
    entries_max: usize,
    compress: bool,
    encrypt: bool,
) -> Result<BackupStats, Error> {

    let pxar_stream = PxarBackupStream::open(
        dir_path.as_ref(),
        device_set,
        verbose,
        skip_lost_and_found,
        catalog,
        exclude_pattern,
        entries_max,
    )?;
    let mut chunk_stream = ChunkStream::new(pxar_stream, chunk_size);

    let (tx, rx) = mpsc::channel(10); // allow to buffer 10 chunks

    let stream = ReceiverStream::new(rx)
        .map_err(Error::from);

    // spawn chunker inside a separate task so that it can run parallel
    tokio::spawn(async move {
        while let Some(v) = chunk_stream.next().await {
            let _ = tx.send(v).await;
        }
    });

    let stats = client
        .upload_stream(previous_manifest, archive_name, stream, "dynamic", None, compress, encrypt)
        .await?;

    Ok(stats)
}

async fn backup_image<P: AsRef<Path>>(
    client: &BackupWriter,
    previous_manifest: Option<Arc<BackupManifest>>,
    image_path: P,
    archive_name: &str,
    image_size: u64,
    chunk_size: Option<usize>,
    compress: bool,
    encrypt: bool,
    _verbose: bool,
) -> Result<BackupStats, Error> {

    let path = image_path.as_ref().to_owned();

    let file = tokio::fs::File::open(path).await?;

    let stream = tokio_util::codec::FramedRead::new(file, tokio_util::codec::BytesCodec::new())
        .map_err(Error::from);

    let stream = FixedChunkStream::new(stream, chunk_size.unwrap_or(4*1024*1024));

    let stats = client
        .upload_stream(previous_manifest, archive_name, stream, "fixed", Some(image_size), compress, encrypt)
        .await?;

    Ok(stats)
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

    let output_format = get_output_format(&param);

    let repo = extract_repository_from_value(&param)?;

    let client = connect(&repo)?;

    let path = format!("api2/json/admin/datastore/{}/groups", repo.store());

    let mut result = client.get(&path, None).await?;

    record_repository(&repo);

    let render_group_path = |_v: &Value, record: &Value| -> Result<String, Error> {
        let item: GroupListItem = serde_json::from_value(record.to_owned())?;
        let group = BackupGroup::new(item.backup_type, item.backup_id);
        Ok(group.group_path().to_str().unwrap().to_owned())
    };

    let render_last_backup = |_v: &Value, record: &Value| -> Result<String, Error> {
        let item: GroupListItem = serde_json::from_value(record.to_owned())?;
        let snapshot = BackupDir::new(item.backup_type, item.backup_id, item.last_backup)?;
        Ok(snapshot.relative_path().to_str().unwrap().to_owned())
    };

    let render_files = |_v: &Value, record: &Value| -> Result<String, Error> {
        let item: GroupListItem = serde_json::from_value(record.to_owned())?;
        Ok(tools::format::render_backup_file_list(&item.files))
    };

    let options = default_table_format_options()
        .sortby("backup-type", false)
        .sortby("backup-id", false)
        .column(ColumnConfig::new("backup-id").renderer(render_group_path).header("group"))
        .column(
            ColumnConfig::new("last-backup")
                .renderer(render_last_backup)
                .header("last snapshot")
                .right_align(false)
        )
        .column(ColumnConfig::new("backup-count"))
        .column(ColumnConfig::new("files").renderer(render_files));

    let mut data: Value = result["data"].take();

    let return_type = &proxmox_backup::api2::admin::datastore::API_METHOD_LIST_GROUPS.returns;

    format_and_print_result_full(&mut data, return_type, &output_format, &options);

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
            },
            "new-owner": {
                type: Authid,
            },
        }
   }
)]
/// Change owner of a backup group
async fn change_backup_owner(group: String, mut param: Value) -> Result<(), Error> {

    let repo = extract_repository_from_value(&param)?;

    let mut client = connect(&repo)?;

    param.as_object_mut().unwrap().remove("repository");

    let group: BackupGroup = group.parse()?;

    param["backup-type"] = group.backup_type().into();
    param["backup-id"] = group.backup_id().into();

    let path = format!("api2/json/admin/datastore/{}/change-owner", repo.store());
    client.post(&path, Some(param)).await?;

    record_repository(&repo);

    Ok(())
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

    let client = connect(&repo)?;
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
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        }
   }
)]
/// Show client and optional server version
async fn api_version(param: Value) -> Result<(), Error> {

    let output_format = get_output_format(&param);

    let mut version_info = json!({
        "client": {
            "version": version::PROXMOX_PKG_VERSION,
            "release": version::PROXMOX_PKG_RELEASE,
            "repoid": version::PROXMOX_PKG_REPOID,
        }
    });

    let repo = extract_repository_from_value(&param);
    if let Ok(repo) = repo {
        let client = connect(&repo)?;

        match client.get("api2/json/version", None).await {
            Ok(mut result) => version_info["server"] = result["data"].take(),
            Err(e) => eprintln!("could not connect to server - {}", e),
        }
    }
    if output_format == "text" {
        println!("client version: {}.{}", version::PROXMOX_PKG_VERSION, version::PROXMOX_PKG_RELEASE);
        if let Some(server) = version_info["server"].as_object() {
            let server_version = server["version"].as_str().unwrap();
            let server_release = server["release"].as_str().unwrap();
            println!("server version: {}.{}", server_version, server_release);
        }
    } else {
        format_and_print_result(&version_info, &output_format);
    }

    Ok(())
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

    let output_format = get_output_format(&param);

    let mut client = connect(&repo)?;

    let path = format!("api2/json/admin/datastore/{}/gc", repo.store());

    let result = client.post(&path, None).await?;

    record_repository(&repo);

    view_task_result(client, result, &output_format).await?;

    Ok(Value::Null)
}

fn spawn_catalog_upload(
    client: Arc<BackupWriter>,
    encrypt: bool,
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
            .upload_stream(None, CATALOG_NAME, catalog_chunk_stream, "dynamic", None, true, encrypt)
            .await;

        if let Err(ref err) = catalog_upload_result {
            eprintln!("catalog upload error - {}", err);
            client.cancel();
        }

        let _ = catalog_result_tx.send(catalog_upload_result);
    });

    Ok((catalog, catalog_result_rx))
}

fn keyfile_parameters(param: &Value) -> Result<(Option<Vec<u8>>, CryptMode), Error> {
    let keyfile = match param.get("keyfile") {
        Some(Value::String(keyfile)) => Some(keyfile),
        Some(_) => bail!("bad --keyfile parameter type"),
        None => None,
    };

    let key_fd = match param.get("keyfd") {
        Some(Value::Number(key_fd)) => Some(
            RawFd::try_from(key_fd
                .as_i64()
                .ok_or_else(|| format_err!("bad key fd: {:?}", key_fd))?
            )
            .map_err(|err| format_err!("bad key fd: {:?}: {}", key_fd, err))?
        ),
        Some(_) => bail!("bad --keyfd parameter type"),
        None => None,
    };

    let crypt_mode: Option<CryptMode> = match param.get("crypt-mode") {
        Some(mode) => Some(serde_json::from_value(mode.clone())?),
        None => None,
    };

    let keydata = match (keyfile, key_fd) {
        (None, None) => None,
        (Some(_), Some(_)) => bail!("--keyfile and --keyfd are mutually exclusive"),
        (Some(keyfile), None) => {
            eprintln!("Using encryption key file: {}", keyfile);
            Some(file_get_contents(keyfile)?)
        },
        (None, Some(fd)) => {
            let input = unsafe { std::fs::File::from_raw_fd(fd) };
            let mut data = Vec::new();
            let _len: usize = { input }.read_to_end(&mut data)
                .map_err(|err| {
                    format_err!("error reading encryption key from fd {}: {}", fd, err)
                })?;
            eprintln!("Using encryption key from file descriptor");
            Some(data)
        }
    };

    Ok(match (keydata, crypt_mode) {
        // no parameters:
        (None, None) => match key::read_optional_default_encryption_key()? {
            Some(key) => {
                eprintln!("Encrypting with default encryption key!");
                (Some(key), CryptMode::Encrypt)
            },
            None => (None, CryptMode::None),
        },

        // just --crypt-mode=none
        (None, Some(CryptMode::None)) => (None, CryptMode::None),

        // just --crypt-mode other than none
        (None, Some(crypt_mode)) => match key::read_optional_default_encryption_key()? {
            None => bail!("--crypt-mode without --keyfile and no default key file available"),
            Some(key) => {
                eprintln!("Encrypting with default encryption key!");
                (Some(key), crypt_mode)
            },
        }

        // just --keyfile
        (Some(key), None) => (Some(key), CryptMode::Encrypt),

        // --keyfile and --crypt-mode=none
        (Some(_), Some(CryptMode::None)) => {
            bail!("--keyfile/--keyfd and --crypt-mode=none are mutually exclusive");
        }

        // --keyfile and --crypt-mode other than none
        (Some(key), Some(crypt_mode)) => (Some(key), crypt_mode),
    })
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
           "all-file-systems": {
               type: Boolean,
               description: "Include all mounted subdirectories.",
               optional: true,
           },
           keyfile: {
               schema: KEYFILE_SCHEMA,
               optional: true,
           },
           "keyfd": {
               schema: KEYFD_SCHEMA,
               optional: true,
           },
           "crypt-mode": {
               type: CryptMode,
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
           "exclude": {
               type: Array,
               description: "List of paths or patterns for matching files to exclude.",
               optional: true,
               items: {
                   type: String,
                   description: "Path or match pattern.",
                }
           },
           "entries-max": {
               type: Integer,
               description: "Max number of entries to hold in memory.",
               optional: true,
               default: proxmox_backup::pxar::ENCODER_MAX_ENTRIES as isize,
           },
           "verbose": {
               type: Boolean,
               description: "Verbose output.",
               optional: true,
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

    let (keydata, crypt_mode) = keyfile_parameters(&param)?;

    let backup_id = param["backup-id"].as_str().unwrap_or(&proxmox::tools::nodename());

    let backup_type = param["backup-type"].as_str().unwrap_or("host");

    let include_dev = param["include-dev"].as_array();

    let entries_max = param["entries-max"].as_u64()
        .unwrap_or(proxmox_backup::pxar::ENCODER_MAX_ENTRIES as u64);

    let empty = Vec::new();
    let exclude_args = param["exclude"].as_array().unwrap_or(&empty);

    let mut pattern_list = Vec::with_capacity(exclude_args.len());
    for entry in exclude_args {
        let entry = entry.as_str().ok_or_else(|| format_err!("Invalid pattern string slice"))?;
        pattern_list.push(
            MatchEntry::parse_pattern(entry, PatternFlag::PATH_NAME, MatchType::Exclude)
                .map_err(|err| format_err!("invalid exclude pattern entry: {}", err))?
        );
    }

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
    let mut target_set = HashSet::new();

    for backupspec in backupspec_list {
        let spec = parse_backup_specification(backupspec.as_str().unwrap())?;
        let filename = &spec.config_string;
        let target = &spec.archive_name;

        if target_set.contains(target) {
            bail!("got target twice: '{}'", target);
        }
        target_set.insert(target.to_string());

        use std::os::unix::fs::FileTypeExt;

        let metadata = std::fs::metadata(filename)
            .map_err(|err| format_err!("unable to access '{}' - {}", filename, err))?;
        let file_type = metadata.file_type();

        match spec.spec_type {
            BackupSpecificationType::PXAR => {
                if !file_type.is_dir() {
                    bail!("got unexpected file type (expected directory)");
                }
                upload_list.push((BackupSpecificationType::PXAR, filename.to_owned(), format!("{}.didx", target), 0));
            }
            BackupSpecificationType::IMAGE => {
                if !(file_type.is_file() || file_type.is_block_device()) {
                    bail!("got unexpected file type (expected file or block device)");
                }

                let size = image_size(&PathBuf::from(filename))?;

                if size == 0 { bail!("got zero-sized file '{}'", filename); }

                upload_list.push((BackupSpecificationType::IMAGE, filename.to_owned(), format!("{}.fidx", target), size));
            }
            BackupSpecificationType::CONFIG => {
                if !file_type.is_file() {
                    bail!("got unexpected file type (expected regular file)");
                }
                upload_list.push((BackupSpecificationType::CONFIG, filename.to_owned(), format!("{}.blob", target), metadata.len()));
            }
            BackupSpecificationType::LOGFILE => {
                if !file_type.is_file() {
                    bail!("got unexpected file type (expected regular file)");
                }
                upload_list.push((BackupSpecificationType::LOGFILE, filename.to_owned(), format!("{}.blob", target), metadata.len()));
            }
        }
    }

    let backup_time = backup_time_opt.unwrap_or_else(|| epoch_i64());

    let client = connect(&repo)?;
    record_repository(&repo);

    println!("Starting backup: {}/{}/{}", backup_type, backup_id, BackupDir::backup_time_to_string(backup_time)?);

    println!("Client name: {}", proxmox::tools::nodename());

    let start_time = std::time::Instant::now();

    println!("Starting backup protocol: {}", strftime_local("%c", epoch_i64())?);

    let (crypt_config, rsa_encrypted_key) = match keydata {
        None => (None, None),
        Some(key) => {
            let (key, created, fingerprint) = decrypt_key(&key, &key::get_encryption_key_password)?;
            println!("Encryption key fingerprint: {}", fingerprint);

            let crypt_config = CryptConfig::new(key.clone())?;

            match key::find_master_pubkey()? {
                Some(ref path) if path.exists() => {
                    let pem_data = file_get_contents(path)?;
                    let rsa = openssl::rsa::Rsa::public_key_from_pem(&pem_data)?;
                    let key_config = KeyConfig {
                        kdf: None,
                        created,
                        modified: proxmox::tools::time::epoch_i64(),
                        data: key.to_vec(),
                        fingerprint: Some(fingerprint),
                    };
                    let enc_key = rsa_encrypt_key_config(rsa, &key_config)?;
                    println!("Master key '{:?}'", path);

                    (Some(Arc::new(crypt_config)), Some(enc_key))
                }
                _ => (Some(Arc::new(crypt_config)), None),
            }
        }
    };

    let client = BackupWriter::start(
        client,
        crypt_config.clone(),
        repo.store(),
        backup_type,
        &backup_id,
        backup_time,
        verbose,
        false
    ).await?;

    let download_previous_manifest = match client.previous_backup_time().await {
        Ok(Some(backup_time)) => {
            println!(
                "Downloading previous manifest ({})",
                strftime_local("%c", backup_time)?
            );
            true
        }
        Ok(None) => {
            println!("No previous manifest available.");
            false
        }
        Err(_) => {
            // Fallback for outdated server, TODO remove/bubble up with 2.0
            true
        }
    };

    let previous_manifest = if download_previous_manifest {
        match client.download_previous_manifest().await {
            Ok(previous_manifest) => {
                match previous_manifest.check_fingerprint(crypt_config.as_ref().map(Arc::as_ref)) {
                    Ok(()) => Some(Arc::new(previous_manifest)),
                    Err(err) => {
                        println!("Couldn't re-use previous manifest - {}", err);
                        None
                    }
                }
            }
            Err(err) => {
                println!("Couldn't download previous manifest - {}", err);
                None
            }
        }
    } else {
        None
    };

    let snapshot = BackupDir::new(backup_type, backup_id, backup_time)?;
    let mut manifest = BackupManifest::new(snapshot);

    let mut catalog = None;
    let mut catalog_result_tx = None;

    for (backup_type, filename, target, size) in upload_list {
        match backup_type {
            BackupSpecificationType::CONFIG => {
                println!("Upload config file '{}' to '{}' as {}", filename, repo, target);
                let stats = client
                    .upload_blob_from_file(&filename, &target, true, crypt_mode == CryptMode::Encrypt)
                    .await?;
                manifest.add_file(target, stats.size, stats.csum, crypt_mode)?;
            }
            BackupSpecificationType::LOGFILE => { // fixme: remove - not needed anymore ?
                println!("Upload log file '{}' to '{}' as {}", filename, repo, target);
                let stats = client
                    .upload_blob_from_file(&filename, &target, true, crypt_mode == CryptMode::Encrypt)
                    .await?;
                manifest.add_file(target, stats.size, stats.csum, crypt_mode)?;
            }
            BackupSpecificationType::PXAR => {
                // start catalog upload on first use
                if catalog.is_none() {
                    let (cat, res) = spawn_catalog_upload(client.clone(), crypt_mode == CryptMode::Encrypt)?;
                    catalog = Some(cat);
                    catalog_result_tx = Some(res);
                }
                let catalog = catalog.as_ref().unwrap();

                println!("Upload directory '{}' to '{}' as {}", filename, repo, target);
                catalog.lock().unwrap().start_directory(std::ffi::CString::new(target.as_str())?.as_c_str())?;
                let stats = backup_directory(
                    &client,
                    previous_manifest.clone(),
                    &filename,
                    &target,
                    chunk_size_opt,
                    devices.clone(),
                    verbose,
                    skip_lost_and_found,
                    catalog.clone(),
                    pattern_list.clone(),
                    entries_max as usize,
                    true,
                    crypt_mode == CryptMode::Encrypt,
                ).await?;
                manifest.add_file(target, stats.size, stats.csum, crypt_mode)?;
                catalog.lock().unwrap().end_directory()?;
            }
            BackupSpecificationType::IMAGE => {
                println!("Upload image '{}' to '{:?}' as {}", filename, repo, target);
                let stats = backup_image(
                    &client,
                    previous_manifest.clone(),
                     &filename,
                    &target,
                    size,
                    chunk_size_opt,
                    true,
                    crypt_mode == CryptMode::Encrypt,
                    verbose,
                ).await?;
                manifest.add_file(target, stats.size, stats.csum, crypt_mode)?;
            }
        }
    }

    // finalize and upload catalog
    if let Some(catalog) = catalog {
        let mutex = Arc::try_unwrap(catalog)
            .map_err(|_| format_err!("unable to get catalog (still used)"))?;
        let mut catalog = mutex.into_inner().unwrap();

        catalog.finish()?;

        drop(catalog); // close upload stream

        if let Some(catalog_result_rx) = catalog_result_tx {
            let stats = catalog_result_rx.await??;
            manifest.add_file(CATALOG_NAME.to_owned(), stats.size, stats.csum, crypt_mode)?;
        }
    }

    if let Some(rsa_encrypted_key) = rsa_encrypted_key {
        let target = ENCRYPTED_KEY_BLOB_NAME;
        println!("Upload RSA encoded key to '{:?}' as {}", repo, target);
        let stats = client
            .upload_blob_from_data(rsa_encrypted_key, target, false, false)
            .await?;
        manifest.add_file(target.to_string(), stats.size, stats.csum, crypt_mode)?;

    }
    // create manifest (index.json)
    // manifests are never encrypted, but include a signature
    let manifest = manifest.to_string(crypt_config.as_ref().map(Arc::as_ref))
        .map_err(|err| format_err!("unable to format manifest - {}", err))?;


    if verbose { println!("Upload index.json to '{}'", repo) };
    client
        .upload_blob_from_data(manifest.into_bytes(), MANIFEST_BLOB_NAME, true, false)
        .await?;

    client.finish().await?;

    let end_time = std::time::Instant::now();
    let elapsed = end_time.duration_since(start_time);
    println!("Duration: {:.2}s", elapsed.as_secs_f64());

    println!("End Time: {}", strftime_local("%c", epoch_i64())?);

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

async fn dump_image<W: Write>(
    client: Arc<BackupReader>,
    crypt_config: Option<Arc<CryptConfig>>,
    crypt_mode: CryptMode,
    index: FixedIndexReader,
    mut writer: W,
    verbose: bool,
) -> Result<(), Error> {

    let most_used = index.find_most_used_chunks(8);

    let chunk_reader = RemoteChunkReader::new(client.clone(), crypt_config, crypt_mode, most_used);

    // Note: we avoid using BufferedFixedReader, because that add an additional buffer/copy
    // and thus slows down reading. Instead, directly use RemoteChunkReader
    let mut per = 0;
    let mut bytes = 0;
    let start_time = std::time::Instant::now();

    for pos in 0..index.index_count() {
        let digest = index.index_digest(pos).unwrap();
        let raw_data = chunk_reader.read_chunk(&digest).await?;
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

fn parse_archive_type(name: &str) -> (String, ArchiveType) {
    if name.ends_with(".didx") || name.ends_with(".fidx") || name.ends_with(".blob") {
        (name.into(), archive_type(name).unwrap())
    } else if name.ends_with(".pxar") {
        (format!("{}.didx", name), ArchiveType::DynamicIndex)
    } else if name.ends_with(".img") {
        (format!("{}.fidx", name), ArchiveType::FixedIndex)
    } else {
        (format!("{}.blob", name), ArchiveType::Blob)
    }
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
           "keyfd": {
               schema: KEYFD_SCHEMA,
               optional: true,
           },
           "crypt-mode": {
               type: CryptMode,
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

    let client = connect(&repo)?;

    record_repository(&repo);

    let path = tools::required_string_param(&param, "snapshot")?;

    let (backup_type, backup_id, backup_time) = if path.matches('/').count() == 1 {
        let group: BackupGroup = path.parse()?;
        api_datastore_latest_snapshot(&client, repo.store(), group).await?
    } else {
        let snapshot: BackupDir = path.parse()?;
        (snapshot.group().backup_type().to_owned(), snapshot.group().backup_id().to_owned(), snapshot.backup_time())
    };

    let target = tools::required_string_param(&param, "target")?;
    let target = if target == "-" { None } else { Some(target) };

    let (keydata, _crypt_mode) = keyfile_parameters(&param)?;

    let crypt_config = match keydata {
        None => None,
        Some(key) => {
            let (key, _, fingerprint) = decrypt_key(&key, &key::get_encryption_key_password)?;
            eprintln!("Encryption key fingerprint: '{}'", fingerprint);
            Some(Arc::new(CryptConfig::new(key)?))
        }
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

    let (archive_name, archive_type) = parse_archive_type(archive_name);

    let (manifest, backup_index_data) = client.download_manifest().await?;

    if archive_name == ENCRYPTED_KEY_BLOB_NAME && crypt_config.is_none() {
        eprintln!("Restoring encrypted key blob without original key - skipping manifest fingerprint check!")
    } else {
        manifest.check_fingerprint(crypt_config.as_ref().map(Arc::as_ref))?;
    }

    if archive_name == MANIFEST_BLOB_NAME {
        if let Some(target) = target {
            replace_file(target, &backup_index_data, CreateOptions::new())?;
        } else {
            let stdout = std::io::stdout();
            let mut writer = stdout.lock();
            writer.write_all(&backup_index_data)
                .map_err(|err| format_err!("unable to pipe data - {}", err))?;
        }

        return Ok(Value::Null);
    }

    let file_info = manifest.lookup_file_info(&archive_name)?;

    if archive_type == ArchiveType::Blob {

        let mut reader = client.download_blob(&manifest, &archive_name).await?;

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

    } else if archive_type == ArchiveType::DynamicIndex {

        let index = client.download_dynamic_index(&manifest, &archive_name).await?;

        let most_used = index.find_most_used_chunks(8);

        let chunk_reader = RemoteChunkReader::new(client.clone(), crypt_config, file_info.chunk_crypt_mode(), most_used);

        let mut reader = BufferedDynamicReader::new(index, chunk_reader);

        if let Some(target) = target {
            proxmox_backup::pxar::extract_archive(
                pxar::decoder::Decoder::from_std(reader)?,
                Path::new(target),
                &[],
                true,
                proxmox_backup::pxar::Flags::DEFAULT,
                allow_existing_dirs,
                |path| {
                    if verbose {
                        println!("{:?}", path);
                    }
                },
                None,
            )
            .map_err(|err| format_err!("error extracting archive - {}", err))?;
        } else {
            let mut writer = std::fs::OpenOptions::new()
                .write(true)
                .open("/dev/stdout")
                .map_err(|err| format_err!("unable to open /dev/stdout - {}", err))?;

            std::io::copy(&mut reader, &mut writer)
                .map_err(|err| format_err!("unable to pipe data - {}", err))?;
        }
    } else if archive_type == ArchiveType::FixedIndex {

        let index = client.download_fixed_index(&manifest, &archive_name).await?;

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

        dump_image(client.clone(), crypt_config.clone(), file_info.chunk_crypt_mode(), index, &mut writer, verbose).await?;
    }

    Ok(Value::Null)
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
            (
                "quiet",
                true,
                &BooleanSchema::new("Minimal output - only show removals.")
                    .schema()
            ),
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

    let mut client = connect(&repo)?;

    let path = format!("api2/json/admin/datastore/{}/prune", repo.store());

    let group = tools::required_string_param(&param, "group")?;
    let group: BackupGroup = group.parse()?;

    let output_format = get_output_format(&param);

    let quiet = param["quiet"].as_bool().unwrap_or(false);

    param.as_object_mut().unwrap().remove("repository");
    param.as_object_mut().unwrap().remove("group");
    param.as_object_mut().unwrap().remove("output-format");
    param.as_object_mut().unwrap().remove("quiet");

    param["backup-type"] = group.backup_type().into();
    param["backup-id"] = group.backup_id().into();

    let mut result = client.post(&path, Some(param)).await?;

    record_repository(&repo);

    let render_snapshot_path = |_v: &Value, record: &Value| -> Result<String, Error> {
        let item: PruneListItem = serde_json::from_value(record.to_owned())?;
        let snapshot = BackupDir::new(item.backup_type, item.backup_id, item.backup_time)?;
        Ok(snapshot.relative_path().to_str().unwrap().to_owned())
    };

    let render_prune_action = |v: &Value, _record: &Value| -> Result<String, Error> {
        Ok(match v.as_bool() {
            Some(true) => "keep",
            Some(false) => "remove",
            None => "unknown",
        }.to_string())
    };

    let options = default_table_format_options()
        .sortby("backup-type", false)
        .sortby("backup-id", false)
        .sortby("backup-time", false)
        .column(ColumnConfig::new("backup-id").renderer(render_snapshot_path).header("snapshot"))
        .column(ColumnConfig::new("backup-time").renderer(tools::format::render_epoch).header("date"))
        .column(ColumnConfig::new("keep").renderer(render_prune_action).header("action"))
        ;

    let return_type = &proxmox_backup::api2::admin::datastore::API_METHOD_PRUNE.returns;

    let mut data = result["data"].take();

    if quiet {
        let list: Vec<Value> = data.as_array().unwrap().iter().filter(|item| {
            item["keep"].as_bool() == Some(false)
        }).map(|v| v.clone()).collect();
        data = list.into();
    }

    format_and_print_result_full(&mut data, return_type, &output_format, &options);

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
   },
    returns: {
        type: StorageStatus,
    },
)]
/// Get repository status.
async fn status(param: Value) -> Result<Value, Error> {

    let repo = extract_repository_from_value(&param)?;

    let output_format = get_output_format(&param);

    let client = connect(&repo)?;

    let path = format!("api2/json/admin/datastore/{}/status", repo.store());

    let mut result = client.get(&path, None).await?;
    let mut data = result["data"].take();

    record_repository(&repo);

    let render_total_percentage = |v: &Value, record: &Value| -> Result<String, Error> {
        let v = v.as_u64().unwrap();
        let total = record["total"].as_u64().unwrap();
        let roundup = total/200;
        let per = ((v+roundup)*100)/total;
        let info = format!(" ({} %)", per);
        Ok(format!("{} {:>8}", v, info))
    };

    let options = default_table_format_options()
        .noheader(true)
        .column(ColumnConfig::new("total").renderer(render_total_percentage))
        .column(ColumnConfig::new("used").renderer(render_total_percentage))
        .column(ColumnConfig::new("avail").renderer(render_total_percentage));

    let return_type = &API_METHOD_STATUS.returns;

    format_and_print_result_full(&mut data, return_type, &output_format, &options);

    Ok(Value::Null)
}

// like get, but simply ignore errors and return Null instead
async fn try_get(repo: &BackupRepository, url: &str) -> Value {

    let fingerprint = std::env::var(ENV_VAR_PBS_FINGERPRINT).ok();
    let password = std::env::var(ENV_VAR_PBS_PASSWORD).ok();

    let options = HttpClientOptions::new()
        .prefix(Some("proxmox-backup".to_string()))
        .password(password)
        .interactive(false)
        .fingerprint(fingerprint)
        .fingerprint_cache(true)
        .ticket_cache(true);

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

pub fn complete_group_or_snapshot(arg: &str, param: &HashMap<String, String>) -> Vec<String> {
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
                if let Ok(snapshot) = BackupDir::new(backup_type, backup_id, backup_time) {
                    result.push(snapshot.relative_path().to_str().unwrap().to_owned());
                }
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

fn complete_archive_name(arg: &str, param: &HashMap<String, String>) -> Vec<String> {
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

fn complete_auth_id(_arg: &str, param: &HashMap<String, String>) -> Vec<String> {
    proxmox_backup::tools::runtime::main(async { complete_auth_id_do(param).await })
}

async fn complete_auth_id_do(param: &HashMap<String, String>) -> Vec<String> {

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

use proxmox_backup::client::RemoteChunkReader;
/// This is a workaround until we have cleaned up the chunk/reader/... infrastructure for better
/// async use!
///
/// Ideally BufferedDynamicReader gets replaced so the LruCache maps to `BroadcastFuture<Chunk>`,
/// so that we can properly access it from multiple threads simultaneously while not issuing
/// duplicate simultaneous reads over http.
pub struct BufferedDynamicReadAt {
    inner: Mutex<BufferedDynamicReader<RemoteChunkReader>>,
}

impl BufferedDynamicReadAt {
    fn new(inner: BufferedDynamicReader<RemoteChunkReader>) -> Self {
        Self {
            inner: Mutex::new(inner),
        }
    }
}

impl ReadAt for BufferedDynamicReadAt {
    fn start_read_at<'a>(
        self: Pin<&'a Self>,
        _cx: &mut Context,
        buf: &'a mut [u8],
        offset: u64,
    ) -> MaybeReady<io::Result<usize>, ReadAtOperation<'a>> {
        MaybeReady::Ready(tokio::task::block_in_place(move || {
            let mut reader = self.inner.lock().unwrap();
            reader.seek(SeekFrom::Start(offset))?;
            Ok(reader.read(buf)?)
        }))
    }

    fn poll_complete<'a>(
        self: Pin<&'a Self>,
        _op: ReadAtOperation<'a>,
    ) -> MaybeReady<io::Result<usize>, ReadAtOperation<'a>> {
        panic!("LocalDynamicReadAt::start_read_at returned Pending");
    }
}

fn main() {

    let backup_cmd_def = CliCommand::new(&API_METHOD_CREATE_BACKUP)
        .arg_param(&["backupspec"])
        .completion_cb("repository", complete_repository)
        .completion_cb("backupspec", complete_backup_source)
        .completion_cb("keyfile", tools::complete_file_name)
        .completion_cb("chunk-size", complete_chunk_size);

    let benchmark_cmd_def = CliCommand::new(&API_METHOD_BENCHMARK)
        .completion_cb("repository", complete_repository)
        .completion_cb("keyfile", tools::complete_file_name);

    let list_cmd_def = CliCommand::new(&API_METHOD_LIST_BACKUP_GROUPS)
        .completion_cb("repository", complete_repository);

    let garbage_collect_cmd_def = CliCommand::new(&API_METHOD_START_GARBAGE_COLLECTION)
        .completion_cb("repository", complete_repository);

    let restore_cmd_def = CliCommand::new(&API_METHOD_RESTORE)
        .arg_param(&["snapshot", "archive-name", "target"])
        .completion_cb("repository", complete_repository)
        .completion_cb("snapshot", complete_group_or_snapshot)
        .completion_cb("archive-name", complete_archive_name)
        .completion_cb("target", tools::complete_file_name);

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

    let version_cmd_def = CliCommand::new(&API_METHOD_API_VERSION)
        .completion_cb("repository", complete_repository);

    let change_owner_cmd_def = CliCommand::new(&API_METHOD_CHANGE_BACKUP_OWNER)
        .arg_param(&["group", "new-owner"])
        .completion_cb("group", complete_backup_group)
        .completion_cb("new-owner",  complete_auth_id)
        .completion_cb("repository", complete_repository);

    let cmd_def = CliCommandMap::new()
        .insert("backup", backup_cmd_def)
        .insert("garbage-collect", garbage_collect_cmd_def)
        .insert("list", list_cmd_def)
        .insert("login", login_cmd_def)
        .insert("logout", logout_cmd_def)
        .insert("prune", prune_cmd_def)
        .insert("restore", restore_cmd_def)
        .insert("snapshot", snapshot_mgtm_cli())
        .insert("status", status_cmd_def)
        .insert("key", key::cli())
        .insert("mount", mount_cmd_def())
        .insert("map", map_cmd_def())
        .insert("unmap", unmap_cmd_def())
        .insert("catalog", catalog_mgmt_cli())
        .insert("task", task_mgmt_cli())
        .insert("version", version_cmd_def)
        .insert("benchmark", benchmark_cmd_def)
        .insert("change-owner", change_owner_cmd_def)

        .alias(&["files"], &["snapshot", "files"])
        .alias(&["forget"], &["snapshot", "forget"])
        .alias(&["upload-log"], &["snapshot", "upload-log"])
        .alias(&["snapshots"], &["snapshot", "list"])
        ;

    let rpcenv = CliEnvironment::new();
    run_cli_command(cmd_def, rpcenv, Some(|future| {
        proxmox_backup::tools::runtime::main(future)
    }));
}
