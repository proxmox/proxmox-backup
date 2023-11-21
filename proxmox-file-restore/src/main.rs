use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{bail, format_err, Error};
use futures::StreamExt;
use serde_json::{json, Value};
use tokio::io::AsyncWriteExt;

use proxmox_compression::zstd::ZstdEncoder;
use proxmox_router::cli::{
    complete_file_name, default_table_format_options, format_and_print_result_full,
    get_output_format, init_cli_logger, run_cli_command, CliCommand, CliCommandMap, CliEnvironment,
    ColumnConfig, OUTPUT_FORMAT,
};
use proxmox_router::{http_err, HttpError};
use proxmox_schema::api;
use proxmox_sys::fs::{create_path, CreateOptions};
use pxar::accessor::aio::Accessor;
use pxar::decoder::aio::Decoder;

use pbs_api_types::{file_restore::FileRestoreFormat, BackupDir, BackupNamespace, CryptMode};
use pbs_client::pxar::{create_tar, create_zip, extract_sub_dir, extract_sub_dir_seq};
use pbs_client::tools::{
    complete_group_or_snapshot, complete_repository, connect, extract_repository_from_value,
    key_source::{
        crypto_parameters_keep_fd, format_key_source, get_encryption_key_password, KEYFD_SCHEMA,
        KEYFILE_SCHEMA,
    },
    REPO_URL_SCHEMA,
};
use pbs_client::{BackupReader, BackupRepository, RemoteChunkReader};
use pbs_datastore::catalog::{ArchiveEntry, CatalogReader, DirEntryAttribute};
use pbs_datastore::dynamic_index::{BufferedDynamicReader, LocalDynamicReadAt};
use pbs_datastore::index::IndexFile;
use pbs_datastore::CATALOG_NAME;
use pbs_key_config::decrypt_key;
use pbs_tools::crypt_config::CryptConfig;

pub mod block_driver;
pub use block_driver::*;

pub mod cpio;

mod block_driver_qemu;
mod qemu_helper;

enum ExtractPath {
    ListArchives,
    Pxar(String, Vec<u8>),
    VM(String, Vec<u8>),
}

fn parse_path(path: String, base64: bool) -> Result<ExtractPath, Error> {
    let mut bytes = if base64 {
        base64::decode(&path)
            .map_err(|err| format_err!("Failed base64-decoding path '{path}' - {err}"))?
    } else {
        path.into_bytes()
    };

    if bytes == b"/" {
        return Ok(ExtractPath::ListArchives);
    }

    while !bytes.is_empty() && bytes[0] == b'/' {
        bytes.remove(0);
    }

    let (file, path) = {
        let slash_pos = bytes.iter().position(|c| *c == b'/').unwrap_or(bytes.len());
        let path = bytes.split_off(slash_pos);
        let file = String::from_utf8(bytes)?;
        (file, path)
    };

    if file.ends_with(".pxar.didx") {
        Ok(ExtractPath::Pxar(file, path))
    } else if file.ends_with(".img.fidx") {
        Ok(ExtractPath::VM(file, path))
    } else {
        bail!("'{file}' is not supported for file-restore");
    }
}

fn keyfile_path(param: &Value) -> Option<String> {
    if let Some(Value::String(keyfile)) = param.get("keyfile") {
        return Some(keyfile.to_owned());
    }

    if let Some(Value::Number(keyfd)) = param.get("keyfd") {
        return Some(format!("/dev/fd/{keyfd}"));
    }

    None
}

async fn list_files(
    repo: BackupRepository,
    namespace: BackupNamespace,
    snapshot: BackupDir,
    path: ExtractPath,
    crypt_config: Option<Arc<CryptConfig>>,
    keyfile: Option<String>,
    driver: Option<BlockDriverType>,
) -> Result<Vec<ArchiveEntry>, Error> {
    let client = connect(&repo)?;
    let client = BackupReader::start(
        &client,
        crypt_config.clone(),
        repo.store(),
        &namespace,
        &snapshot,
        true,
    )
    .await?;

    let (manifest, _) = client.download_manifest().await?;
    manifest.check_fingerprint(crypt_config.as_ref().map(Arc::as_ref))?;

    match path {
        ExtractPath::ListArchives => {
            let mut entries = vec![];
            for file in manifest.files() {
                if !file.filename.ends_with(".pxar.didx") && !file.filename.ends_with(".img.fidx") {
                    continue;
                }
                let path = format!("/{}", file.filename);
                let attr = if file.filename.ends_with(".pxar.didx") {
                    // a pxar file is a file archive, so it's root is also a directory root
                    Some(&DirEntryAttribute::Directory { start: 0 })
                } else {
                    None
                };
                entries.push(ArchiveEntry::new_with_size(
                    path.as_bytes(),
                    attr,
                    Some(file.size),
                ));
            }

            Ok(entries)
        }
        ExtractPath::Pxar(file, mut path) => {
            let index = client
                .download_dynamic_index(&manifest, CATALOG_NAME)
                .await?;
            let most_used = index.find_most_used_chunks(8);
            let file_info = manifest.lookup_file_info(CATALOG_NAME)?;
            let chunk_reader = RemoteChunkReader::new(
                client.clone(),
                crypt_config,
                file_info.chunk_crypt_mode(),
                most_used,
            );
            let reader = BufferedDynamicReader::new(index, chunk_reader);
            let mut catalog_reader = CatalogReader::new(reader);

            let mut fullpath = file.into_bytes();
            fullpath.append(&mut path);

            catalog_reader.list_dir_contents(&fullpath)
        }
        ExtractPath::VM(file, path) => {
            let details = SnapRestoreDetails {
                manifest,
                repo,
                namespace,
                snapshot,
                keyfile,
            };
            data_list(driver, details, file, path).await
        }
    }
}

#[api(
    input: {
        properties: {
            repository: {
                schema: REPO_URL_SCHEMA,
                optional: true,
            },
            ns: {
                type: BackupNamespace,
                optional: true,
            },
            snapshot: {
                type: String,
                description: "Group/Snapshot path.",
            },
            "path": {
                description: "(Sub-)Path to list.",
                type: String,
            },
            "base64": {
                type: Boolean,
                description: "If set, 'path' will be interpreted as base64 encoded.",
                optional: true,
                default: false,
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
            "driver": {
                type: BlockDriverType,
                optional: true,
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
            "timeout": {
                type: Integer,
                description: "Defines the maximum time the call can should take.",
                minimum: 1,
                optional: true,
            },
        }
    },
    returns: {
        description: "A list of elements under the given path",
        type: Array,
        items: {
            type: ArchiveEntry,
        }
    }
)]
/// List a directory from a backup snapshot.
async fn list(
    ns: Option<BackupNamespace>,
    snapshot: String,
    path: String,
    base64: bool,
    timeout: Option<u64>,
    param: Value,
) -> Result<(), Error> {
    let repo = extract_repository_from_value(&param)?;
    let ns = ns.unwrap_or_default();
    let snapshot: BackupDir = snapshot.parse()?;
    let path = parse_path(path, base64)?;

    let keyfile = keyfile_path(&param);
    let crypto = crypto_parameters_keep_fd(&param)?;
    let crypt_config = match crypto.enc_key {
        None => None,
        Some(ref key) => {
            let (key, _, _) =
                decrypt_key(&key.key, &get_encryption_key_password).map_err(|err| {
                    log::error!("{}", format_key_source(&key.source, "encryption"));
                    err
                })?;
            Some(Arc::new(CryptConfig::new(key)?))
        }
    };

    let driver: Option<BlockDriverType> = match param.get("driver") {
        Some(drv) => Some(serde::Deserialize::deserialize(drv)?),
        None => None,
    };

    let result = if let Some(timeout) = timeout {
        match tokio::time::timeout(
            std::time::Duration::from_secs(timeout),
            list_files(repo, ns, snapshot, path, crypt_config, keyfile, driver),
        )
        .await
        {
            Ok(res) => res,
            Err(_) => Err(http_err!(SERVICE_UNAVAILABLE, "list not finished in time")),
        }
    } else {
        list_files(repo, ns, snapshot, path, crypt_config, keyfile, driver).await
    };

    let output_format = get_output_format(&param);

    if let Err(err) = result {
        if &output_format == "text" {
            return Err(err);
        }
        let (msg, code) = match err.downcast_ref::<HttpError>() {
            Some(HttpError { code, message }) => (message.clone(), Some(code)),
            None => (err.to_string(), None),
        };
        let mut json_err = json!({
            "message": msg,
        });
        if let Some(code) = code {
            json_err["code"] = Value::from(code.as_u16());
        }
        match output_format.as_ref() {
            "json-pretty" => println!("{}", serde_json::to_string_pretty(&json_err)?),
            _ => println!("{}", serde_json::to_string(&json_err)?),
        }
        return Ok(());
    }

    let options = default_table_format_options()
        .sortby("type", false)
        .sortby("text", false)
        .column(ColumnConfig::new("type"))
        .column(ColumnConfig::new("text").header("name"))
        .column(ColumnConfig::new("mtime").header("last modified"))
        .column(ColumnConfig::new("size"));

    let output_format = get_output_format(&param);
    format_and_print_result_full(
        &mut json!(result.unwrap()),
        &API_METHOD_LIST.returns,
        &output_format,
        &options,
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
            ns: {
                type: BackupNamespace,
                optional: true,
            },
            snapshot: {
                type: String,
                description: "Group/Snapshot path.",
            },
            "path": {
                description: "Path to restore. Directories will be restored as archive files if extracted to stdout.",
                type: String,
            },
            "format": {
                type: FileRestoreFormat,
                optional: true,
            },
            "zstd": {
                type: bool,
                description: "If true, output will be zstd compressed.",
                optional: true,
                default: false,
            },
            "base64": {
                type: Boolean,
                description: "If set, 'path' will be interpreted as base64 encoded.",
                optional: true,
                default: false,
            },
            target: {
                type: String,
                optional: true,
                description: "Target directory path. Use '-' to write to standard output.",
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
            verbose: {
                type: Boolean,
                description: "Print verbose information",
                optional: true,
                default: false,
            },
            "driver": {
                type: BlockDriverType,
                optional: true,
            },
        }
    }
)]
/// Restore files from a backup snapshot.
#[allow(clippy::too_many_arguments)]
async fn extract(
    ns: Option<BackupNamespace>,
    snapshot: String,
    path: String,
    base64: bool,
    target: Option<String>,
    format: Option<FileRestoreFormat>,
    zstd: bool,
    param: Value,
) -> Result<(), Error> {
    let repo = extract_repository_from_value(&param)?;
    let namespace = ns.unwrap_or_default();
    let snapshot: BackupDir = snapshot.parse()?;
    let orig_path = path;
    let path = parse_path(orig_path.clone(), base64)?;

    let target = match target {
        Some(target) if target == "-" => None,
        Some(target) => Some(PathBuf::from(target)),
        None => Some(std::env::current_dir()?),
    };

    let keyfile = keyfile_path(&param);
    let crypto = crypto_parameters_keep_fd(&param)?;
    let crypt_config = match crypto.enc_key {
        None => None,
        Some(ref key) => {
            let (key, _, _) =
                decrypt_key(&key.key, &get_encryption_key_password).map_err(|err| {
                    log::error!("{}", format_key_source(&key.source, "encryption"));
                    err
                })?;
            Some(Arc::new(CryptConfig::new(key)?))
        }
    };

    let client = connect(&repo)?;
    let client = BackupReader::start(
        &client,
        crypt_config.clone(),
        repo.store(),
        &namespace,
        &snapshot,
        true,
    )
    .await?;
    let (manifest, _) = client.download_manifest().await?;

    match path {
        ExtractPath::Pxar(archive_name, path) => {
            let file_info = manifest.lookup_file_info(&archive_name)?;
            let index = client
                .download_dynamic_index(&manifest, &archive_name)
                .await?;
            let most_used = index.find_most_used_chunks(8);
            let chunk_reader = RemoteChunkReader::new(
                client.clone(),
                crypt_config,
                file_info.chunk_crypt_mode(),
                most_used,
            );
            let reader = BufferedDynamicReader::new(index, chunk_reader);

            let archive_size = reader.archive_size();
            let reader = LocalDynamicReadAt::new(reader);
            let decoder = Accessor::new(reader, archive_size).await?;
            extract_to_target(decoder, &path, target, format, zstd).await?;
        }
        ExtractPath::VM(file, path) => {
            let details = SnapRestoreDetails {
                manifest,
                repo,
                namespace,
                snapshot,
                keyfile,
            };
            let driver: Option<BlockDriverType> = match param.get("driver") {
                Some(drv) => Some(serde::Deserialize::deserialize(drv)?),
                None => None,
            };

            if let Some(mut target) = target {
                let reader = data_extract(
                    driver,
                    details,
                    file,
                    path.clone(),
                    Some(FileRestoreFormat::Pxar),
                    false,
                )
                .await?;
                let decoder = Decoder::from_tokio(reader).await?;
                extract_sub_dir_seq(&target, decoder).await?;

                // we extracted a .pxarexclude-cli file auto-generated by the VM when encoding the
                // archive, this file is of no use for the user, so try to remove it
                target.push(".pxarexclude-cli");
                std::fs::remove_file(target).map_err(|err| {
                    format_err!("unable to remove temporary .pxarexclude-cli file - {err}")
                })?;
            } else {
                let mut reader =
                    data_extract(driver, details, file, path.clone(), format, zstd).await?;
                tokio::io::copy(&mut reader, &mut tokio::io::stdout()).await?;
            }
        }
        _ => {
            bail!("cannot extract '{orig_path}'");
        }
    }

    Ok(())
}

async fn extract_to_target<T>(
    decoder: Accessor<T>,
    path: &[u8],
    target: Option<PathBuf>,
    format: Option<FileRestoreFormat>,
    zstd: bool,
) -> Result<(), Error>
where
    T: pxar::accessor::ReadAt + Clone + Send + Sync + Unpin + 'static,
{
    let path = if path.is_empty() { b"/" } else { path };
    let path = OsStr::from_bytes(path);

    if let Some(target) = target {
        extract_sub_dir(target, decoder, path).await?;
    } else {
        extract_archive(decoder, path, format, zstd).await?;
    }

    Ok(())
}

async fn extract_archive<T>(
    decoder: Accessor<T>,
    path: &OsStr,
    format: Option<FileRestoreFormat>,
    zstd: bool,
) -> Result<(), Error>
where
    T: pxar::accessor::ReadAt + Clone + Send + Sync + Unpin + 'static,
{
    let path = path.to_owned();
    let root = decoder.open_root().await?;
    let file = root
        .lookup(&path)
        .await?
        .ok_or_else(|| format_err!("error opening '{:?}'", &path))?;

    let (mut writer, mut reader) = tokio::io::duplex(1024 * 1024);
    if file.is_regular_file() {
        match format {
            Some(FileRestoreFormat::Plain) | None => {}
            _ => bail!("cannot extract single files as archive"),
        }
        tokio::spawn(
            async move { tokio::io::copy(&mut file.contents().await?, &mut writer).await },
        );
    } else {
        match format {
            Some(FileRestoreFormat::Pxar) => {
                bail!("pxar target not supported for pxar source");
            }
            Some(FileRestoreFormat::Plain) => {
                bail!("plain file not supported for non-regular files");
            }
            Some(FileRestoreFormat::Zip) | None => {
                tokio::spawn(create_zip(writer, decoder, path));
            }
            Some(FileRestoreFormat::Tar) => {
                tokio::spawn(create_tar(writer, decoder, path));
            }
        }
    }

    if zstd {
        let mut zstdstream = ZstdEncoder::new(tokio_util::io::ReaderStream::new(reader))?;
        let mut stdout = tokio::io::stdout();
        while let Some(buf) = zstdstream.next().await {
            let buf = buf?;
            stdout.write_all(&buf).await?;
        }
    } else {
        tokio::io::copy(&mut reader, &mut tokio::io::stdout()).await?;
    }

    Ok(())
}

fn main() {
    let loglevel = match qemu_helper::debug_mode() {
        true => "debug",
        false => "info",
    };
    init_cli_logger("PBS_LOG", loglevel);

    let list_cmd_def = CliCommand::new(&API_METHOD_LIST)
        .arg_param(&["snapshot", "path"])
        .completion_cb("repository", complete_repository)
        .completion_cb("snapshot", complete_group_or_snapshot);

    let restore_cmd_def = CliCommand::new(&API_METHOD_EXTRACT)
        .arg_param(&["snapshot", "path", "target"])
        .completion_cb("repository", complete_repository)
        .completion_cb("snapshot", complete_group_or_snapshot)
        .completion_cb("target", complete_file_name);

    let status_cmd_def = CliCommand::new(&API_METHOD_STATUS);
    let stop_cmd_def = CliCommand::new(&API_METHOD_STOP)
        .arg_param(&["name"])
        .completion_cb("name", complete_block_driver_ids);

    let cmd_def = CliCommandMap::new()
        .insert("list", list_cmd_def)
        .insert("extract", restore_cmd_def)
        .insert("status", status_cmd_def)
        .insert("stop", stop_cmd_def);

    let rpcenv = CliEnvironment::new();
    run_cli_command(
        cmd_def,
        rpcenv,
        Some(|future| proxmox_async::runtime::main(future)),
    );
}

/// Returns a runtime dir owned by the current user.
/// Note that XDG_RUNTIME_DIR is not always available, especially for non-login users like
/// "www-data", so we use a custom one in /run/proxmox-backup/<uid> instead.
pub fn get_user_run_dir() -> Result<std::path::PathBuf, Error> {
    let uid = nix::unistd::Uid::current();
    let mut path: std::path::PathBuf = pbs_buildcfg::PROXMOX_BACKUP_RUN_DIR.into();
    path.push(uid.to_string());
    create_run_dir()?;
    std::fs::create_dir_all(&path)?;
    Ok(path)
}

/// FIXME: proxmox-file-restore should not depend on this!
fn create_run_dir() -> Result<(), Error> {
    let backup_user = backup_user()?;
    let opts = CreateOptions::new()
        .owner(backup_user.uid)
        .group(backup_user.gid);
    let _: bool = create_path(pbs_buildcfg::PROXMOX_BACKUP_RUN_DIR_M!(), None, Some(opts))?;
    Ok(())
}

/// Return User info for the 'backup' user (``getpwnam_r(3)``)
pub fn backup_user() -> Result<nix::unistd::User, Error> {
    nix::unistd::User::from_name(pbs_buildcfg::BACKUP_USER_NAME)?.ok_or_else(|| {
        format_err!(
            "Unable to lookup '{}' user.",
            pbs_buildcfg::BACKUP_USER_NAME
        )
    })
}
