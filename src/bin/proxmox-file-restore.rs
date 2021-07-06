use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{bail, format_err, Error};
use serde_json::{json, Value};

use proxmox::api::{
    api,
    cli::{
        default_table_format_options, format_and_print_result_full, get_output_format,
        run_cli_command, CliCommand, CliCommandMap, CliEnvironment, ColumnConfig, OUTPUT_FORMAT,
    },
};
use pxar::accessor::aio::Accessor;
use pxar::decoder::aio::Decoder;

use proxmox_backup::api2::{helpers, types::ArchiveEntry};
use proxmox_backup::backup::{
    decrypt_key, BackupDir, BufferedDynamicReader, CatalogReader, CryptConfig, CryptMode,
    DirEntryAttribute, IndexFile, LocalDynamicReadAt, CATALOG_NAME,
};
use proxmox_backup::client::{BackupReader, RemoteChunkReader};
use proxmox_backup::pxar::{create_zip, extract_sub_dir, extract_sub_dir_seq};
use proxmox_backup::tools;

// use "pub" so rust doesn't complain about "unused" functions in the module
pub mod proxmox_client_tools;
use proxmox_client_tools::{
    complete_group_or_snapshot, complete_repository, connect, extract_repository_from_value,
    key_source::{
        crypto_parameters_keep_fd, format_key_source, get_encryption_key_password, KEYFD_SCHEMA,
        KEYFILE_SCHEMA,
    },
    REPO_URL_SCHEMA,
};

mod proxmox_file_restore;
use proxmox_file_restore::*;

enum ExtractPath {
    ListArchives,
    Pxar(String, Vec<u8>),
    VM(String, Vec<u8>),
}

fn parse_path(path: String, base64: bool) -> Result<ExtractPath, Error> {
    let mut bytes = if base64 {
        base64::decode(&path)
            .map_err(|err| format_err!("Failed base64-decoding path '{}' - {}", path, err))?
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
        bail!("'{}' is not supported for file-restore", file);
    }
}

fn keyfile_path(param: &Value) -> Option<String> {
    if let Some(Value::String(keyfile)) = param.get("keyfile") {
        return Some(keyfile.to_owned());
    }

    if let Some(Value::Number(keyfd)) = param.get("keyfd") {
        return Some(format!("/dev/fd/{}", keyfd));
    }

    None
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
           "path": {
               description: "Path to restore. Directories will be restored as .zip files.",
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
    snapshot: String,
    path: String,
    base64: bool,
    param: Value,
) -> Result<(), Error> {
    let repo = extract_repository_from_value(&param)?;
    let snapshot: BackupDir = snapshot.parse()?;
    let path = parse_path(path, base64)?;

    let keyfile = keyfile_path(&param);
    let crypto = crypto_parameters_keep_fd(&param)?;
    let crypt_config = match crypto.enc_key {
        None => None,
        Some(ref key) => {
            let (key, _, _) =
                decrypt_key(&key.key, &get_encryption_key_password).map_err(|err| {
                    eprintln!("{}", format_key_source(&key.source, "encryption"));
                    err
                })?;
            Some(Arc::new(CryptConfig::new(key)?))
        }
    };

    let client = connect(&repo)?;
    let client = BackupReader::start(
        client,
        crypt_config.clone(),
        repo.store(),
        &snapshot.group().backup_type(),
        &snapshot.group().backup_id(),
        snapshot.backup_time(),
        true,
    )
    .await?;

    let (manifest, _) = client.download_manifest().await?;
    manifest.check_fingerprint(crypt_config.as_ref().map(Arc::as_ref))?;

    let result = match path {
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
                entries.push(ArchiveEntry::new_with_size(path.as_bytes(), attr, Some(file.size)));
            }

            Ok(entries)
        }
        ExtractPath::Pxar(file, mut path) => {
            let index = client
                .download_dynamic_index(&manifest, CATALOG_NAME)
                .await?;
            let most_used = index.find_most_used_chunks(8);
            let file_info = manifest.lookup_file_info(&CATALOG_NAME)?;
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

            helpers::list_dir_content(&mut catalog_reader, &fullpath)
        }
        ExtractPath::VM(file, path) => {
            let details = SnapRestoreDetails {
                manifest,
                repo,
                snapshot,
                keyfile,
            };
            let driver: Option<BlockDriverType> = match param.get("driver") {
                Some(drv) => Some(serde_json::from_value(drv.clone())?),
                None => None,
            };
            data_list(driver, details, file, path).await
        }
    }?;

    let options = default_table_format_options()
        .sortby("type", false)
        .sortby("text", false)
        .column(ColumnConfig::new("type"))
        .column(ColumnConfig::new("text").header("name"))
        .column(ColumnConfig::new("mtime").header("last modified"))
        .column(ColumnConfig::new("size"));

    let output_format = get_output_format(&param);
    format_and_print_result_full(
        &mut json!(result),
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
           snapshot: {
               type: String,
               description: "Group/Snapshot path.",
           },
           "path": {
               description: "Path to restore. Directories will be restored as .zip files if extracted to stdout.",
               type: String,
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
async fn extract(
    snapshot: String,
    path: String,
    base64: bool,
    target: Option<String>,
    verbose: bool,
    param: Value,
) -> Result<(), Error> {
    let repo = extract_repository_from_value(&param)?;
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
                    eprintln!("{}", format_key_source(&key.source, "encryption"));
                    err
                })?;
            Some(Arc::new(CryptConfig::new(key)?))
        }
    };

    let client = connect(&repo)?;
    let client = BackupReader::start(
        client,
        crypt_config.clone(),
        repo.store(),
        &snapshot.group().backup_type(),
        &snapshot.group().backup_id(),
        snapshot.backup_time(),
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
            extract_to_target(decoder, &path, target, verbose).await?;
        }
        ExtractPath::VM(file, path) => {
            let details = SnapRestoreDetails {
                manifest,
                repo,
                snapshot,
                keyfile,
            };
            let driver: Option<BlockDriverType> = match param.get("driver") {
                Some(drv) => Some(serde_json::from_value(drv.clone())?),
                None => None,
            };

            if let Some(mut target) = target {
                let reader = data_extract(driver, details, file, path.clone(), true).await?;
                let decoder = Decoder::from_tokio(reader).await?;
                extract_sub_dir_seq(&target, decoder, verbose).await?;

                // we extracted a .pxarexclude-cli file auto-generated by the VM when encoding the
                // archive, this file is of no use for the user, so try to remove it
                target.push(".pxarexclude-cli");
                std::fs::remove_file(target).map_err(|e| {
                    format_err!("unable to remove temporary .pxarexclude-cli file - {}", e)
                })?;
            } else {
                let mut reader = data_extract(driver, details, file, path.clone(), false).await?;
                tokio::io::copy(&mut reader, &mut tokio::io::stdout()).await?;
            }
        }
        _ => {
            bail!("cannot extract '{}'", orig_path);
        }
    }

    Ok(())
}

async fn extract_to_target<T>(
    decoder: Accessor<T>,
    path: &[u8],
    target: Option<PathBuf>,
    verbose: bool,
) -> Result<(), Error>
where
    T: pxar::accessor::ReadAt + Clone + Send + Sync + Unpin + 'static,
{
    let path = if path.is_empty() { b"/" } else { path };

    let root = decoder.open_root().await?;
    let file = root
        .lookup(OsStr::from_bytes(path))
        .await?
        .ok_or_else(|| format_err!("error opening '{:?}'", path))?;

    if let Some(target) = target {
        extract_sub_dir(target, decoder, OsStr::from_bytes(path), verbose).await?;
    } else {
        match file.kind() {
            pxar::EntryKind::File { .. } => {
                tokio::io::copy(&mut file.contents().await?, &mut tokio::io::stdout()).await?;
            }
            _ => {
                create_zip(
                    tokio::io::stdout(),
                    decoder,
                    OsStr::from_bytes(path),
                    verbose,
                )
                .await?;
            }
        }
    }

    Ok(())
}

fn main() {
    let list_cmd_def = CliCommand::new(&API_METHOD_LIST)
        .arg_param(&["snapshot", "path"])
        .completion_cb("repository", complete_repository)
        .completion_cb("snapshot", complete_group_or_snapshot);

    let restore_cmd_def = CliCommand::new(&API_METHOD_EXTRACT)
        .arg_param(&["snapshot", "path", "target"])
        .completion_cb("repository", complete_repository)
        .completion_cb("snapshot", complete_group_or_snapshot)
        .completion_cb("target", tools::complete_file_name);

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
        Some(|future| pbs_runtime::main(future)),
    );
}
