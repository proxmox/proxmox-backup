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

use pbs_api_types::{BackupNamespace, BackupPart, HumanByte};
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
    let potentially_modified: HashMap<&OsStr, (&FileEntry, &FileEntry)> = files_in_a
        .iter()
        .filter_map(|(path, entry_a)| {
            files_in_b
                .get(path)
                .map(|entry_b| (path.as_os_str(), (entry_a, entry_b)))
        })
        .collect();

    // ... so we compare the file metadata/contents to narrow the selection down to files
    // which where *really* modified.
    let modified_files = compare_files(potentially_modified).await?;

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
    files: HashMap<&'a OsStr, (&'a FileEntry, &'a FileEntry)>,
) -> Result<HashMap<&'a OsStr, (&'a FileEntry, ChangedProperties)>, Error> {
    let mut modified_files = HashMap::new();

    for (path, (entry_a, entry_b)) in files {
        if let Some(changed) = compare_file(entry_a, entry_b).await? {
            modified_files.insert(path, (entry_b, changed));
        }
    }

    Ok(modified_files)
}

async fn compare_file(
    file_a: &FileEntry,
    file_b: &FileEntry,
) -> Result<Option<ChangedProperties>, Error> {
    let mut changed = ChangedProperties::default();

    changed.set_from_metadata(file_a, file_b);

    match (file_a.kind(), file_b.kind()) {
        (EntryKind::Symlink(a), EntryKind::Symlink(b)) => {
            // Check whether the link target has changed.
            changed.content = a.as_os_str() != b.as_os_str();
        }
        (EntryKind::Hardlink(a), EntryKind::Hardlink(b)) => {
            // Check whether the link target has changed.
            changed.content = a.as_os_str() != b.as_os_str();
        }
        (EntryKind::Device(a), EntryKind::Device(b)) => {
            changed.content = a.major != b.major
                || a.minor != b.minor
                || file_a.metadata().stat.is_blockdev() != file_b.metadata().stat.is_blockdev();
        }
        (EntryKind::File { size: size_a, .. }, EntryKind::File { size: size_b, .. }) => {
            if size_a != size_b {
                changed.size = true;
                changed.content = true;
            };
        }
        (EntryKind::Directory, EntryKind::Directory) => {}
        (EntryKind::Socket, EntryKind::Socket) => {}
        (EntryKind::Fifo, EntryKind::Fifo) => {}
        (_, _) => {
            changed.entry_type = true;
        }
    }

    if changed.any() {
        Ok(Some(changed))
    } else {
        Ok(None)
    }
}

#[derive(Copy, Clone, Default)]
struct ChangedProperties {
    entry_type: bool,
    mtime: bool,
    acl: bool,
    xattrs: bool,
    fcaps: bool,
    quota_project_id: bool,
    mode: bool,
    flags: bool,
    uid: bool,
    gid: bool,
    size: bool,
    content: bool,
}

impl ChangedProperties {
    fn set_from_metadata(&mut self, file_a: &FileEntry, file_b: &FileEntry) {
        let a = file_a.metadata();
        let b = file_b.metadata();

        self.acl = a.acl != b.acl;
        self.xattrs = a.xattrs != b.xattrs;
        self.fcaps = a.fcaps != b.fcaps;
        self.quota_project_id = a.quota_project_id != b.quota_project_id;
        self.mode = a.stat.mode != b.stat.mode;
        self.flags = a.stat.flags != b.stat.flags;
        self.uid = a.stat.uid != b.stat.uid;
        self.gid = a.stat.gid != b.stat.gid;
        self.mtime = a.stat.mtime != b.stat.mtime;
    }

    fn any(&self) -> bool {
        self.entry_type
            || self.mtime
            || self.acl
            || self.xattrs
            || self.fcaps
            || self.quota_project_id
            || self.mode
            || self.flags
            || self.uid
            || self.gid
            || self.content
    }
}

fn change_indicator(changed: bool) -> &'static str {
    if changed {
        "*"
    } else {
        " "
    }
}

fn format_filesize(entry: &FileEntry, changed: bool) -> String {
    if let Some(size) = entry.file_size() {
        format!(
            "{}{:.1}",
            change_indicator(changed),
            HumanByte::new_decimal(size as f64)
        )
    } else {
        String::new()
    }
}

fn format_mtime(entry: &FileEntry, changed: bool) -> String {
    let mtime = &entry.metadata().stat.mtime;

    let format = if changed { "*%F %T" } else { " %F %T" };

    proxmox_time::strftime_local(format, mtime.secs).unwrap_or_default()
}

fn format_mode(entry: &FileEntry, changed: bool) -> String {
    let mode = entry.metadata().stat.mode & 0o7777;
    format!("{}{:o}", change_indicator(changed), mode)
}

fn format_entry_type(entry: &FileEntry, changed: bool) -> String {
    let kind = match entry.kind() {
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

    format!("{}{}", change_indicator(changed), kind)
}

fn format_uid(entry: &FileEntry, changed: bool) -> String {
    format!("{}{}", change_indicator(changed), entry.metadata().stat.uid)
}

fn format_gid(entry: &FileEntry, changed: bool) -> String {
    format!("{}{}", change_indicator(changed), entry.metadata().stat.gid)
}

fn format_file_name(entry: &FileEntry, changed: bool) -> String {
    format!(
        "{}{}",
        change_indicator(changed),
        entry.file_name().to_string_lossy()
    )
}

/// Display a sorted list of added, modified, deleted files.
fn show_file_list(
    added: &HashMap<&OsStr, &FileEntry>,
    deleted: &HashMap<&OsStr, &FileEntry>,
    modified: &HashMap<&OsStr, (&FileEntry, ChangedProperties)>,
) {
    let mut all: Vec<&OsStr> = Vec::new();

    all.extend(added.keys());
    all.extend(deleted.keys());
    all.extend(modified.keys());

    all.sort();

    for file in all {
        let (op, entry, changed) = if let Some(entry) = added.get(file) {
            ("A", entry, ChangedProperties::default())
        } else if let Some(entry) = deleted.get(file) {
            ("D", entry, ChangedProperties::default())
        } else if let Some((entry, changed)) = modified.get(file) {
            ("M", entry, *changed)
        } else {
            unreachable!();
        };

        let entry_type = format_entry_type(entry, changed.entry_type);
        let uid = format_uid(entry, changed.uid);
        let gid = format_gid(entry, changed.gid);
        let mode = format_mode(entry, changed.mode);
        let size = format_filesize(entry, changed.size);
        let mtime = format_mtime(entry, changed.mtime);
        let name = format_file_name(entry, changed.content);

        println!("{op} {entry_type:>2} {mode:>5} {uid:>6} {gid:>6} {size:>10} {mtime:11} {name}");
    }
}
