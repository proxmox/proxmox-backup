use std::collections::{HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::iter::FromIterator;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{bail, Context as AnyhowContext, Error};
use futures::future::BoxFuture;
use futures::FutureExt;

use proxmox_router::cli::{CliCommand, CliCommandMap, CommandLineInterface};
use proxmox_schema::api;

use pbs_api_types::{BackupNamespace, BackupPart};
use pbs_client::tools::key_source::{
    crypto_parameters, format_key_source, get_encryption_key_password, KEYFD_SCHEMA,
};
use pbs_client::tools::{
    complete_archive_name, complete_group_or_snapshot, connect, extract_repository_from_value,
    REPO_URL_SCHEMA,
};
use pbs_client::{BackupReader, BackupRepository, RemoteChunkReader};
use pbs_config::key_config::decrypt_key;
use pbs_datastore::dynamic_index::{BufferedDynamicReader, DynamicIndexReader, LocalDynamicReadAt};
use pbs_datastore::index::IndexFile;
use pbs_tools::crypt_config::CryptConfig;
use pbs_tools::json::required_string_param;
use pxar::accessor::ReadAt;
use pxar::EntryKind;
use serde_json::Value;

type ChunkDigest = [u8; 32];
type FileEntry = pxar::accessor::aio::FileEntry<Arc<dyn ReadAt + Send + Sync>>;
type Accessor = pxar::accessor::aio::Accessor<Arc<dyn ReadAt + Send + Sync>>;
type Directory = pxar::accessor::aio::Directory<Arc<dyn ReadAt + Send + Sync>>;

pub fn diff_commands() -> CommandLineInterface {
    let cmd_def = CliCommandMap::new().insert(
        "archive",
        CliCommand::new(&API_METHOD_DIFF_ARCHIVE_CMD)
            .arg_param(&["prev-snapshot", "snapshot", "archive-name"])
            .completion_cb("prev-snapshot", complete_group_or_snapshot)
            .completion_cb("snapshot", complete_group_or_snapshot)
            .completion_cb("archive-name", complete_archive_name),
    );

    cmd_def.into()
}

#[api(
    input: {
        properties: {
            "ns": {
                type: BackupNamespace,
                optional: true,
            },
            "prev-snapshot": {
                description: "Path for the first snapshot.",
                type: String,
            },
            "snapshot": {
                description: "Path for the second snapshot.",
                type: String,
            },
            "archive-name": {
                description: "Name of the .pxar archive",
                type: String,
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
            "keyfd": {
                schema: KEYFD_SCHEMA,
                optional: true,
            },
        }
    }
)]
/// Diff an archive in two snapshots. The command will output a list of added, modified and deleted files.
/// For modified files, only the file metadata (e.g. mtime, size, etc.) will be considered. The actual
/// file contents will not be compared.
async fn diff_archive_cmd(param: Value) -> Result<(), Error> {
    let repo = extract_repository_from_value(&param)?;
    let snapshot_a = required_string_param(&param, "prev-snapshot")?;
    let snapshot_b = required_string_param(&param, "snapshot")?;
    let archive_name = required_string_param(&param, "archive-name")?;

    let namespace = match param.get("ns") {
        Some(Value::String(ns)) => ns.parse()?,
        Some(_) => bail!("invalid namespace parameter"),
        None => BackupNamespace::root(),
    };

    let crypto = crypto_parameters(&param)?;

    let crypt_config = match crypto.enc_key {
        None => None,
        Some(key) => {
            let (key, _created, _fingerprint) = decrypt_key(&key.key, &get_encryption_key_password)
                .map_err(|err| {
                    log::error!("{}", format_key_source(&key.source, "encryption"));
                    err
                })?;
            let crypt_config = CryptConfig::new(key)?;
            Some(Arc::new(crypt_config))
        }
    };

    let repo_params = RepoParams {
        repo,
        crypt_config,
        namespace,
    };

    if archive_name.ends_with(".pxar") {
        let file_name = format!("{}.didx", archive_name);
        diff_archive(snapshot_a, snapshot_b, &file_name, &repo_params).await?;
    } else {
        bail!("Only .pxar files are supported");
    }

    Ok(())
}

async fn diff_archive(
    snapshot_a: &str,
    snapshot_b: &str,
    file_name: &str,
    repo_params: &RepoParams,
) -> Result<(), Error> {
    let (index_a, accessor_a) = open_dynamic_index(snapshot_a, file_name, repo_params).await?;
    let (index_b, accessor_b) = open_dynamic_index(snapshot_b, file_name, repo_params).await?;

    // vecs of chunk digests, in their correct order
    let chunks_a = chunk_digests_for_index(&index_a);
    let chunks_b = chunk_digests_for_index(&index_b);

    // sets of chunk digests, 'cause we want to perform set operations
    let chunk_set_a: HashSet<&ChunkDigest> = HashSet::from_iter(chunks_a.iter().copied());
    let chunk_set_b: HashSet<&ChunkDigest> = HashSet::from_iter(chunks_b.iter().copied());

    // Symmetric difference between both sets -
    // content stored in those chunks was either added, modified or deleted
    let chunk_sym_diff: HashSet<&ChunkDigest> = chunk_set_a
        .symmetric_difference(&chunk_set_b)
        .copied()
        .collect();

    // Figure out which files are stored in which chunks
    let files_in_a = files_in_chunk_set(&chunks_a, &accessor_a, &index_a, &chunk_sym_diff).await?;
    let files_in_b = files_in_chunk_set(&chunks_b, &accessor_b, &index_b, &chunk_sym_diff).await?;

    // If file in A but not in B --> deleted
    let deleted_files: HashMap<&OsStr, &FileEntry> = files_in_a
        .iter()
        .filter(|(path, _)| !files_in_b.contains_key(*path))
        .map(|(path, entry)| (path.as_os_str(), entry))
        .collect();

    // If file in B but not in A --> added
    let added_files: HashMap<&OsStr, &FileEntry> = files_in_b
        .iter()
        .filter(|(path, _)| !files_in_a.contains_key(*path))
        .map(|(path, entry)| (path.as_os_str(), entry))
        .collect();

    // If file is present in both snapshots, it *might* be modified, but does not have to be.
    // If another, unmodified file resides in the same chunk as an actually modified one,
    // it will also show up as modified here...
    let potentially_modified: HashMap<&OsStr, &FileEntry> = files_in_a
        .iter()
        .filter(|(path, _)| files_in_b.contains_key(*path))
        .map(|(path, entry)| (path.as_os_str(), entry))
        .collect();

    // ... so we compare the file metadata/contents to narrow the selection down to files
    // which where *really* modified.
    let modified_files = compare_files(&files_in_a, &files_in_b, potentially_modified).await?;

    show_file_list(&added_files, &deleted_files, &modified_files);

    Ok(())
}

struct RepoParams {
    repo: BackupRepository,
    crypt_config: Option<Arc<CryptConfig>>,
    namespace: BackupNamespace,
}

async fn open_dynamic_index(
    snapshot: &str,
    archive_name: &str,
    params: &RepoParams,
) -> Result<(DynamicIndexReader, Accessor), Error> {
    let backup_reader = create_backup_reader(snapshot, params).await?;

    let (manifest, _) = backup_reader.download_manifest().await?;
    manifest.check_fingerprint(params.crypt_config.as_ref().map(Arc::as_ref))?;

    let index = backup_reader
        .download_dynamic_index(&manifest, archive_name)
        .await?;
    let most_used = index.find_most_used_chunks(8);

    let lookup_index = backup_reader
        .download_dynamic_index(&manifest, archive_name)
        .await?;

    let file_info = manifest.lookup_file_info(archive_name)?;
    let chunk_reader = RemoteChunkReader::new(
        backup_reader.clone(),
        params.crypt_config.clone(),
        file_info.chunk_crypt_mode(),
        most_used,
    );

    let reader = BufferedDynamicReader::new(index, chunk_reader);
    let archive_size = reader.archive_size();
    let reader: Arc<dyn ReadAt + Send + Sync> = Arc::new(LocalDynamicReadAt::new(reader));
    let accessor = Accessor::new(reader, archive_size).await?;

    Ok((lookup_index, accessor))
}

async fn create_backup_reader(
    snapshot: &str,
    params: &RepoParams,
) -> Result<Arc<BackupReader>, Error> {
    let backup_dir = match snapshot.parse::<BackupPart>()? {
        BackupPart::Dir(dir) => dir,
        BackupPart::Group(_group) => {
            bail!("A full snapshot path must be provided.");
        }
    };
    let client = connect(&params.repo)?;
    let backup_reader = BackupReader::start(
        client,
        params.crypt_config.clone(),
        params.repo.store(),
        &params.namespace,
        &backup_dir,
        false,
    )
    .await?;
    Ok(backup_reader)
}

/// Get a list of chunk digests for an index file.
fn chunk_digests_for_index(index: &dyn IndexFile) -> Vec<&ChunkDigest> {
    let mut all_chunks = Vec::new();

    for i in 0..index.index_count() {
        let digest = index
            .index_digest(i)
            .expect("Invalid chunk index - index corrupted?");
        all_chunks.push(digest);
    }

    all_chunks
}

/// Compute which files are contained in a given chunk set.
async fn files_in_chunk_set<'c, 'f>(
    chunk_list: &[&'c ChunkDigest],
    accessor: &'f Accessor,
    index: &'f DynamicIndexReader,
    chunk_set: &HashSet<&'c ChunkDigest>,
) -> Result<HashMap<OsString, FileEntry>, Error> {
    let path = PathBuf::new();
    let root = accessor.open_root().await?;

    visit_directory(&root, index, &path, chunk_list, chunk_set).await
}

/// Recursively visits directories in .pxar archive and create a
/// map "digest --> set of contained files"
fn visit_directory<'f, 'c>(
    directory: &'f Directory,
    index: &'f DynamicIndexReader,
    path: &'f Path,
    chunk_list: &'f [&'c ChunkDigest],
    chunk_diff: &'f HashSet<&'c ChunkDigest>,
) -> BoxFuture<'f, Result<HashMap<OsString, FileEntry>, Error>> {
    async move {
        let mut entries: HashMap<OsString, FileEntry> = HashMap::new();

        let mut iter = directory.read_dir();

        while let Some(entry) = iter.next().await {
            let entry = entry?.decode_entry().await?;
            let range = &entry.entry_range_info().entry_range;

            let first_chunk = index
                .chunk_from_offset(range.start)
                .context("Invalid offest")?
                .0;
            let last_chunk = index
                .chunk_from_offset(range.end)
                .context("Invalid offset")?
                .0;

            if entry.is_dir() {
                let new_dir = entry.enter_directory().await?;

                for chunk_index in first_chunk..=last_chunk {
                    // Check if any chunk of the serialized directory is in
                    // set off modified chunks (symmetric difference).
                    // If not, we can skip the directory entirely and save a lot of time.

                    let digest = chunk_list.get(chunk_index).context("Invalid chunk index")?;

                    if chunk_diff.get(digest).is_some() {
                        let dir_path = path.join(entry.file_name());

                        entries.extend(
                            visit_directory(&new_dir, index, &dir_path, chunk_list, chunk_diff)
                                .await?
                                .into_iter(),
                        );
                        break;
                    }
                }
            }

            let file_path = path.join(entry.file_name());

            for chunk_index in first_chunk..=last_chunk {
                let digest = chunk_list.get(chunk_index).context("Invalid chunk index")?;

                if chunk_diff.get(digest).is_some() {
                    // files.insert(file_path.clone().into_os_string());
                    entries.insert(file_path.into_os_string(), entry);
                    break;
                }
            }
        }

        Ok(entries)
    }
    .boxed()
}

/// Check if files were actually modified
async fn compare_files<'a>(
    entries_a: &HashMap<OsString, FileEntry>,
    entries_b: &HashMap<OsString, FileEntry>,
    files: HashMap<&'a OsStr, &'a FileEntry>,
) -> Result<HashMap<&'a OsStr, &'a FileEntry>, Error> {
    let mut modified_files = HashMap::new();

    for (path, entry) in files {
        let p = path.to_os_string();
        let file_a = entries_a.get(&p).context("File entry not in map")?;
        let file_b = entries_b.get(&p).context("File entry not in map")?;

        if !compare_file(file_a, file_b).await {
            modified_files.insert(path, entry);
        }
    }

    Ok(modified_files)
}

async fn compare_file(file_a: &FileEntry, file_b: &FileEntry) -> bool {
    if file_a.metadata() != file_b.metadata() {
        // Check if mtime, permissions, ACLs, etc. have changed - if they have changed, we consider
        // the file as modified.
        return false;
    }

    match (file_a.kind(), file_b.kind()) {
        (EntryKind::Symlink(a), EntryKind::Symlink(b)) => {
            // Check whether the link target has changed.
            a.as_os_str() == b.as_os_str()
        }
        (EntryKind::Hardlink(a), EntryKind::Hardlink(b)) => {
            // Check whether the link target has changed.
            a.as_os_str() == b.as_os_str()
        }
        (EntryKind::Device(a), EntryKind::Device(b)) => a.major == b.major && a.minor == b.minor,
        (EntryKind::Socket, EntryKind::Socket) => true,
        (EntryKind::Fifo, EntryKind::Fifo) => true,
        (EntryKind::File { size: size_a, .. }, EntryKind::File { size: size_b, .. }) => {
            // At this point we know that all metadata including mtime is
            // the same. To speed things up, we consider the files as equal if they also have
            // the same size.
            // If one were completely paranoid, one could compare the actual file contents,
            // but this decreases performance drastically.
            size_a == size_b
        }
        (EntryKind::Directory, EntryKind::Directory) => true,
        (_, _) => false, // Kind has changed, so we of course consider it modified.
    }
}

/// Display a sorted list of added, modified, deleted files.
fn show_file_list(
    added: &HashMap<&OsStr, &FileEntry>,
    deleted: &HashMap<&OsStr, &FileEntry>,
    modified: &HashMap<&OsStr, &FileEntry>,
) {
    let mut all: Vec<&OsStr> = Vec::new();

    all.extend(added.keys());
    all.extend(deleted.keys());
    all.extend(modified.keys());

    all.sort();

    for file in all {
        let (op, entry) = if let Some(entry) = added.get(file) {
            ("A", *entry)
        } else if let Some(entry) = deleted.get(file) {
            ("D", *entry)
        } else if let Some(entry) = modified.get(file) {
            ("M", *entry)
        } else {
            unreachable!();
        };

        let entry_kind = match entry.kind() {
            EntryKind::Symlink(_) => "l",
            EntryKind::Hardlink(_) => "h",
            EntryKind::Device(_) if entry.metadata().stat.is_blockdev() => "b",
            EntryKind::Device(_) => "c",
            EntryKind::Socket => "s",
            EntryKind::Fifo => "p",
            EntryKind::File { .. } => "f",
            EntryKind::Directory => "d",
            _ => " ",
        };

        println!("{} {} {}", op, entry_kind, file.to_string_lossy());
    }
}
