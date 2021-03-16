use std::path::Path;
use std::sync::Arc;
use std::os::unix::io::{AsRawFd, FromRawFd};
use std::fs::File;

use anyhow::{bail, Error};
use nix::dir::Dir;

use crate::{
    tools::fs::lock_dir_noblock_shared,
    backup::{
        DataStore,
        BackupDir,
        ArchiveType,
        IndexFile,
        FixedIndexReader,
        DynamicIndexReader,
        MANIFEST_BLOB_NAME,
        CLIENT_LOG_BLOB_NAME,
        archive_type,
    },
};

/// Helper to access the contents of a datastore backup snapshot
///
/// This make it easy to iterate over all used chunks and files.
pub struct SnapshotReader {
    snapshot: BackupDir,
    datastore_name: String,
    file_list: Vec<String>,
    locked_dir: Dir,
}

impl SnapshotReader {

    /// Lock snapshot, reads the manifest and returns a new instance
    pub fn new(datastore: Arc<DataStore>, snapshot: BackupDir) -> Result<Self, Error> {

        let snapshot_path = datastore.snapshot_path(&snapshot);

        let locked_dir = lock_dir_noblock_shared(
            &snapshot_path,
            "snapshot",
            "locked by another operation")?;

        let datastore_name = datastore.name().to_string();

        let manifest = match datastore.load_manifest(&snapshot) {
            Ok((manifest, _)) => manifest,
            Err(err) => {
                bail!("manifest load error on datastore '{}' snapshot '{}' - {}",
                      datastore_name, snapshot, err);
            }
        };

        let mut client_log_path = snapshot_path;
        client_log_path.push(CLIENT_LOG_BLOB_NAME);

        let mut file_list = Vec::new();
        file_list.push(MANIFEST_BLOB_NAME.to_string());
        for item in manifest.files() { file_list.push(item.filename.clone()); }
        if client_log_path.exists() {
            file_list.push(CLIENT_LOG_BLOB_NAME.to_string());
        }

        Ok(Self { snapshot, datastore_name, file_list, locked_dir })
    }

    /// Return the snapshot directory
    pub fn snapshot(&self) -> &BackupDir {
        &self.snapshot
    }

    /// Return the datastore name
    pub fn datastore_name(&self) -> &str {
        &self.datastore_name
    }

    /// Returns the list of files the snapshot refers to.
    pub fn file_list(&self) -> &Vec<String> {
        &self.file_list
    }

    /// Opens a file inside the snapshot (using openat) for reading
    pub fn open_file(&self, filename: &str) -> Result<File, Error> {
        let raw_fd = nix::fcntl::openat(
            self.locked_dir.as_raw_fd(),
            Path::new(filename),
            nix::fcntl::OFlag::O_RDONLY,
            nix::sys::stat::Mode::empty(),
        )?;
        let file = unsafe { File::from_raw_fd(raw_fd) };
        Ok(file)
    }

    /// Returns an iterator for all used chunks.
    pub fn chunk_iterator(&self) -> Result<SnapshotChunkIterator, Error> {
        SnapshotChunkIterator::new(&self)
    }
}

/// Iterates over all chunks used by a backup snapshot
///
/// Note: The iterator returns a `Result`, and the iterator state is
/// undefined after the first error. So it make no sense to continue
/// iteration after the first error.
pub struct SnapshotChunkIterator<'a> {
    snapshot_reader: &'a SnapshotReader,
    todo_list: Vec<String>,
    current_index: Option<(Arc<Box<dyn IndexFile>>, usize)>,
}

impl <'a> Iterator for SnapshotChunkIterator<'a> {
    type Item = Result<[u8; 32], Error>;

    fn next(&mut self) -> Option<Self::Item> {
        proxmox::try_block!({
            loop {
                if self.current_index.is_none() {
                    if let Some(filename) = self.todo_list.pop() {
                        let file = self.snapshot_reader.open_file(&filename)?;
                        let index: Box<dyn IndexFile> = match archive_type(&filename)? {
                            ArchiveType::FixedIndex => Box::new(FixedIndexReader::new(file)?),
                            ArchiveType::DynamicIndex => Box::new(DynamicIndexReader::new(file)?),
                            _ => bail!("SnapshotChunkIterator: got unknown file type - internal error"),
                        };
                        self.current_index = Some((Arc::new(index), 0));
                    } else {
                        return Ok(None);
                    }
                }
                let (index, pos) = self.current_index.take().unwrap();
                if pos < index.index_count() {
                    let digest = *index.index_digest(pos).unwrap();
                    self.current_index = Some((index, pos + 1));
                    return Ok(Some(digest));
                } else {
                    // pop next index
                }
            }
        }).transpose()
    }
}

impl <'a> SnapshotChunkIterator<'a> {

    pub fn new(snapshot_reader: &'a SnapshotReader) -> Result<Self, Error> {

        let mut todo_list = Vec::new();

        for filename in snapshot_reader.file_list() {
            match archive_type(filename)? {
                ArchiveType::FixedIndex | ArchiveType::DynamicIndex => {
                    todo_list.push(filename.to_owned());
                },
                ArchiveType::Blob => { /* no chunks, do nothing */ },
            }
        }

        Ok(Self { snapshot_reader, todo_list, current_index: None })
    }
}
