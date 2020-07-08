use std::os::unix::fs::OpenOptionsExt;
use std::io::{Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{bail, format_err, Error};
use serde_json::Value;

use proxmox::api::{api, cli::*};

use proxmox_backup::tools;

use proxmox_backup::client::*;

use crate::{
    REPO_URL_SCHEMA,
    extract_repository_from_value,
    record_repository,
    api_datastore_latest_snapshot,
    complete_repository,
    complete_backup_snapshot,
    complete_group_or_snapshot,
    complete_pxar_archive_name,
    connect,
    BackupDir,
    BackupGroup,
    BufferedDynamicReader,
    BufferedDynamicReadAt,
    CatalogReader,
    CATALOG_NAME,
    CryptConfig,
    DynamicIndexReader,
    IndexFile,
    Shell,
};

use proxmox_backup::backup::load_and_decrypt_key;

use crate::key::get_encryption_key_password;

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
    let snapshot: BackupDir = path.parse()?;

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

    let (manifest, _) = client.download_manifest().await?;

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
        let group: BackupGroup = path.parse()?;
        api_datastore_latest_snapshot(&client, repo.store(), group).await?
    } else {
        let snapshot: BackupDir = path.parse()?;
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

    let mut tmpfile = std::fs::OpenOptions::new()
        .write(true)
        .read(true)
        .custom_flags(libc::O_TMPFILE)
        .open("/tmp")?;

    let (manifest, _) = client.download_manifest().await?;

    let index = client.download_dynamic_index(&manifest, &server_archive_name).await?;
    let most_used = index.find_most_used_chunks(8);
    let chunk_reader = RemoteChunkReader::new(client.clone(), crypt_config.clone(), most_used);
    let reader = BufferedDynamicReader::new(index, chunk_reader);
    let archive_size = reader.archive_size();
    let reader: proxmox_backup::pxar::fuse::Reader =
        Arc::new(BufferedDynamicReadAt::new(reader));
    let decoder = proxmox_backup::pxar::fuse::Accessor::new(reader, archive_size).await?;

    client.download(CATALOG_NAME, &mut tmpfile).await?;
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
    ).await?;

    println!("Starting interactive shell");
    state.shell().await?;

    record_repository(&repo);

    Ok(())
}

pub fn catalog_mgmt_cli() -> CliCommandMap {
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
