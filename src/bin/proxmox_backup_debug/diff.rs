use std::collections::{HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::io::{IsTerminal, Write};
use std::iter::FromIterator;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{bail, Context as AnyhowContext, Error};
use futures::future::BoxFuture;
use futures::FutureExt;

use proxmox_human_byte::HumanByte;
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
use pbs_datastore::dynamic_index::{BufferedDynamicReader, DynamicIndexReader, LocalDynamicReadAt};
use pbs_datastore::index::IndexFile;
use pbs_key_config::decrypt_key;
use pbs_tools::crypt_config::CryptConfig;
use pxar::accessor::ReadAt;
use pxar::EntryKind;
use serde::Deserialize;
use serde_json::Value;

use termcolor::{Color, ColorChoice, ColorSpec, StandardStream, WriteColor};
use tokio::io::AsyncReadExt;

type ChunkDigest = [u8; 32];
type FileEntry = pxar::accessor::aio::FileEntry<Arc<dyn ReadAt + Send + Sync>>;
type Accessor = pxar::accessor::aio::Accessor<Arc<dyn ReadAt + Send + Sync>>;
type Directory = pxar::accessor::aio::Directory<Arc<dyn ReadAt + Send + Sync>>;

const BUFFERSIZE: usize = 4096;

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
            "compare-content": {
                optional: true,
                type: bool,
                default: false,
                description: "Compare file content rather than solely relying on mtime for detecting modified files.",
            },
            "color": {
                optional: true,
                type: ColorMode,
            }
        }
    }
)]
/// Diff an archive in two snapshots. The command will output a list of added, modified and deleted files.
/// For modified files, the file metadata (e.g. mode, uid, gid, size, etc.) will be considered. For detecting
/// modification of file content, only mtime will be used by default. If the --compare-content flag is provided,
/// mtime is ignored and file content will be compared.
async fn diff_archive_cmd(
    prev_snapshot: String,
    snapshot: String,
    archive_name: String,
    compare_content: bool,
    color: Option<ColorMode>,
    ns: Option<BackupNamespace>,
    param: Value,
) -> Result<(), Error> {
    let repo = extract_repository_from_value(&param)?;

    let color = color.unwrap_or_default();
    let namespace = ns.unwrap_or_else(BackupNamespace::root);

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

    let output_params = OutputParams { color };

    if archive_name.ends_with(".pxar") {
        let file_name = format!("{}.didx", archive_name);
        diff_archive(
            &prev_snapshot,
            &snapshot,
            &file_name,
            &repo_params,
            compare_content,
            &output_params,
        )
        .await?;
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
    compare_contents: bool,
    output_params: &OutputParams,
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
    let modified_files = compare_files(potentially_modified, compare_contents).await?;

    show_file_list(&added_files, &deleted_files, &modified_files, output_params)?;

    Ok(())
}

#[api]
#[derive(Default, Deserialize)]
#[serde(rename_all = "lowercase")]
/// Color output options
enum ColorMode {
    /// Always output colors
    Always,
    /// Output colors if STDOUT is a tty and neither of TERM=dumb or NO_COLOR is set
    #[default]
    Auto,
    /// Never output colors
    Never,
}

struct RepoParams {
    repo: BackupRepository,
    crypt_config: Option<Arc<CryptConfig>>,
    namespace: BackupNamespace,
}

struct OutputParams {
    color: ColorMode,
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
        &client,
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
                .context("Invalid offset")?
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
    compare_contents: bool,
) -> Result<HashMap<&'a OsStr, (&'a FileEntry, ChangedProperties)>, Error> {
    let mut modified_files = HashMap::new();

    for (path, (entry_a, entry_b)) in files {
        if let Some(changed) = compare_file(entry_a, entry_b, compare_contents).await? {
            modified_files.insert(path, (entry_b, changed));
        }
    }

    Ok(modified_files)
}

async fn compare_file(
    file_a: &FileEntry,
    file_b: &FileEntry,
    compare_contents: bool,
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
            changed.size = size_a != size_b;

            if compare_contents {
                if changed.size {
                    changed.content = true;
                } else {
                    let content_identical = compare_file_contents(file_a, file_b).await?;
                    if content_identical && !changed.any_without_mtime() {
                        // If the content is identical and nothing, excluding mtime,
                        // has changed, we don't consider the entry as modified.
                        changed.mtime = false;
                    }

                    changed.content = !content_identical;
                }
            }
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

async fn compare_file_contents(file_a: &FileEntry, file_b: &FileEntry) -> Result<bool, Error> {
    let mut contents_a = file_a.contents().await?;
    let mut contents_b = file_b.contents().await?;

    compare_readers(&mut contents_a, &mut contents_b).await
}

async fn compare_readers<T>(reader_a: &mut T, reader_b: &mut T) -> Result<bool, Error>
where
    T: AsyncReadExt + Unpin,
{
    let mut buf_a = Box::new([0u8; BUFFERSIZE]);
    let mut buf_b = Box::new([0u8; BUFFERSIZE]);

    loop {
        // Put the both read calls into their own async blocks, otherwise
        // tokio::try_join! in combination with our #[api] macro leads to some
        // weird `higher-order lifetime error`
        let read_fut_a = async { reader_a.read(buf_a.as_mut_slice()).await };
        let read_fut_b = async { reader_b.read(buf_b.as_mut_slice()).await };

        let (bytes_read_a, bytes_read_b) = tokio::try_join!(read_fut_a, read_fut_b)?;

        if bytes_read_a != bytes_read_b {
            return Ok(false);
        }

        if bytes_read_a == 0 {
            break;
        }

        if buf_a[..bytes_read_a] != buf_b[..bytes_read_b] {
            return Ok(false);
        }
    }

    Ok(true)
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
        self.any_without_mtime() || self.mtime
    }

    fn any_without_mtime(&self) -> bool {
        self.entry_type
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

enum FileOperation {
    Added,
    Modified,
    Deleted,
}

struct ColumnWidths {
    operation: usize,
    entry_type: usize,
    uid: usize,
    gid: usize,
    mode: usize,
    filesize: usize,
    mtime: usize,
}

impl Default for ColumnWidths {
    fn default() -> Self {
        Self {
            operation: 1,
            entry_type: 2,
            uid: 6,
            gid: 6,
            mode: 6,
            filesize: 10,
            mtime: 11,
        }
    }
}

struct FileEntryPrinter {
    stream: StandardStream,
    column_widths: ColumnWidths,
    changed_color: Color,
}

impl FileEntryPrinter {
    pub fn new(output_params: &OutputParams) -> Self {
        let color_choice = match output_params.color {
            ColorMode::Always => ColorChoice::Always,
            ColorMode::Auto => {
                if std::io::stdout().is_terminal() {
                    // Show colors unless `TERM=dumb` or `NO_COLOR` is set.
                    ColorChoice::Auto
                } else {
                    ColorChoice::Never
                }
            }
            ColorMode::Never => ColorChoice::Never,
        };

        let stdout = StandardStream::stdout(color_choice);

        Self {
            stream: stdout,
            column_widths: ColumnWidths::default(),
            changed_color: Color::Yellow,
        }
    }

    fn change_indicator(&self, changed: bool) -> &'static str {
        if changed {
            "*"
        } else {
            " "
        }
    }

    fn set_color_if_changed(&mut self, changed: bool) -> Result<(), Error> {
        if changed {
            self.stream
                .set_color(ColorSpec::new().set_fg(Some(self.changed_color)))?;
        }

        Ok(())
    }

    fn write_operation(&mut self, op: FileOperation) -> Result<(), Error> {
        let (text, color) = match op {
            FileOperation::Added => ("A", Color::Green),
            FileOperation::Modified => ("M", Color::Yellow),
            FileOperation::Deleted => ("D", Color::Red),
        };

        self.stream
            .set_color(ColorSpec::new().set_fg(Some(color)))?;

        write!(
            self.stream,
            "{text:>width$}",
            width = self.column_widths.operation,
        )?;

        self.stream.reset()?;

        Ok(())
    }

    fn write_filesize(&mut self, entry: &FileEntry, changed: bool) -> Result<(), Error> {
        let output = if let Some(size) = entry.file_size() {
            format!(
                "{}{:.1}",
                self.change_indicator(changed),
                HumanByte::new_decimal(size as f64)
            )
        } else {
            String::new()
        };

        self.set_color_if_changed(changed)?;
        write!(
            self.stream,
            "{output:>width$}",
            width = self.column_widths.filesize,
        )?;
        self.stream.reset()?;

        Ok(())
    }

    fn write_mtime(&mut self, entry: &FileEntry, changed: bool) -> Result<(), Error> {
        let mtime = &entry.metadata().stat.mtime;

        let mut format = self.change_indicator(changed).to_owned();
        format.push_str("%F %T");

        let output = proxmox_time::strftime_local(&format, mtime.secs).unwrap_or_default();

        self.set_color_if_changed(changed)?;
        write!(
            self.stream,
            "{output:>width$}",
            width = self.column_widths.mtime,
        )?;
        self.stream.reset()?;

        Ok(())
    }

    fn write_mode(&mut self, entry: &FileEntry, changed: bool) -> Result<(), Error> {
        let mode = entry.metadata().stat.mode & 0o7777;
        let output = format!("{}{:o}", self.change_indicator(changed), mode);

        self.set_color_if_changed(changed)?;
        write!(
            self.stream,
            "{output:>width$}",
            width = self.column_widths.mode,
        )?;
        self.stream.reset()?;

        Ok(())
    }

    fn write_entry_type(&mut self, entry: &FileEntry, changed: bool) -> Result<(), Error> {
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

        let output = format!("{}{}", self.change_indicator(changed), kind);

        self.set_color_if_changed(changed)?;
        write!(
            self.stream,
            "{output:>width$}",
            width = self.column_widths.entry_type,
        )?;
        self.stream.reset()?;

        Ok(())
    }

    fn write_uid(&mut self, entry: &FileEntry, changed: bool) -> Result<(), Error> {
        let output = format!(
            "{}{}",
            self.change_indicator(changed),
            entry.metadata().stat.uid
        );

        self.set_color_if_changed(changed)?;
        write!(
            self.stream,
            "{output:>width$}",
            width = self.column_widths.uid,
        )?;
        self.stream.reset()?;
        Ok(())
    }

    fn write_gid(&mut self, entry: &FileEntry, changed: bool) -> Result<(), Error> {
        let output = format!(
            "{}{}",
            self.change_indicator(changed),
            entry.metadata().stat.gid
        );

        self.set_color_if_changed(changed)?;
        write!(
            self.stream,
            "{output:>width$}",
            width = self.column_widths.gid,
        )?;
        self.stream.reset()?;
        Ok(())
    }

    fn write_file_name(&mut self, entry: &FileEntry, changed: bool) -> Result<(), Error> {
        self.set_color_if_changed(changed)?;
        write!(
            self.stream,
            "{}{}",
            self.change_indicator(changed),
            entry.file_name().to_string_lossy()
        )?;
        self.stream.reset()?;

        Ok(())
    }

    fn write_column_seperator(&mut self) -> Result<(), Error> {
        write!(self.stream, " ")?;
        Ok(())
    }

    /// Print a file entry, including `changed` indicators and column separators
    pub fn print_file_entry(
        &mut self,
        entry: &FileEntry,
        changed: &ChangedProperties,
        operation: FileOperation,
    ) -> Result<(), Error> {
        self.write_operation(operation)?;
        self.write_column_seperator()?;

        self.write_entry_type(entry, changed.entry_type)?;
        self.write_column_seperator()?;

        self.write_uid(entry, changed.uid)?;
        self.write_column_seperator()?;

        self.write_gid(entry, changed.gid)?;
        self.write_column_seperator()?;

        self.write_mode(entry, changed.mode)?;
        self.write_column_seperator()?;

        self.write_filesize(entry, changed.size)?;
        self.write_column_seperator()?;

        self.write_mtime(entry, changed.mtime)?;
        self.write_column_seperator()?;

        self.write_file_name(entry, changed.content)?;
        writeln!(self.stream)?;

        Ok(())
    }
}

/// Display a sorted list of added, modified, deleted files.
fn show_file_list(
    added: &HashMap<&OsStr, &FileEntry>,
    deleted: &HashMap<&OsStr, &FileEntry>,
    modified: &HashMap<&OsStr, (&FileEntry, ChangedProperties)>,
    output_params: &OutputParams,
) -> Result<(), Error> {
    let mut all: Vec<&OsStr> = Vec::new();

    all.extend(added.keys());
    all.extend(deleted.keys());
    all.extend(modified.keys());

    all.sort();

    let mut printer = FileEntryPrinter::new(output_params);

    for file in all {
        let (operation, entry, changed) = if let Some(entry) = added.get(file) {
            (FileOperation::Added, entry, ChangedProperties::default())
        } else if let Some(entry) = deleted.get(file) {
            (FileOperation::Deleted, entry, ChangedProperties::default())
        } else if let Some((entry, changed)) = modified.get(file) {
            (FileOperation::Modified, entry, *changed)
        } else {
            unreachable!();
        };

        printer.print_file_entry(entry, &changed, operation)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::{
        io::Cursor,
        pin::Pin,
        task::{Context, Poll},
    };
    use tokio::io::{AsyncRead, ReadBuf};

    struct MockedAsyncReader(Cursor<Vec<u8>>);

    impl AsyncRead for MockedAsyncReader {
        fn poll_read(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            read_buf: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            let mut buf = vec![0u8; 100];

            let res = std::io::Read::read(&mut self.get_mut().0, &mut buf);

            if let Ok(bytes) = res {
                read_buf.put_slice(&buf[..bytes]);
            }

            Poll::Ready(res.map(|_| ()))
        }
    }

    #[test]
    fn test_do_compare_file_contents() {
        fn compare(a: Vec<u8>, b: Vec<u8>) -> Result<bool, Error> {
            let mut mock_a = MockedAsyncReader(Cursor::new(a));
            let mut mock_b = MockedAsyncReader(Cursor::new(b));

            proxmox_async::runtime::block_on(compare_readers(&mut mock_a, &mut mock_b))
        }

        assert!(matches!(compare(vec![0; 15], vec![0; 15]), Ok(true)));
        assert!(matches!(compare(vec![0; 15], vec![0; 14]), Ok(false)));
        assert!(matches!(compare(vec![0; 15], vec![1; 15]), Ok(false)));

        let mut buf = vec![1u8; 2 * BUFFERSIZE];
        buf[BUFFERSIZE] = 0;
        assert!(matches!(compare(vec![1u8; 2 * BUFFERSIZE], buf), Ok(false)));

        let mut buf = vec![1u8; 2 * BUFFERSIZE];
        buf[2 * BUFFERSIZE - 1] = 0;
        assert!(matches!(compare(vec![1u8; 2 * BUFFERSIZE], buf), Ok(false)));
    }
}
