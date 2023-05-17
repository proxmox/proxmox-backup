use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{bail, format_err, Error};

use pbs_api_types::{DatastoreFSyncLevel, GarbageCollectionStatus};
use proxmox_io::ReadExt;
use proxmox_sys::fs::{create_dir, create_path, file_type_from_file_stat, CreateOptions};
use proxmox_sys::process_locker::{
    ProcessLockExclusiveGuard, ProcessLockSharedGuard, ProcessLocker,
};
use proxmox_sys::task_log;
use proxmox_sys::WorkerTaskContext;

use crate::file_formats::{
    COMPRESSED_BLOB_MAGIC_1_0, ENCRYPTED_BLOB_MAGIC_1_0, UNCOMPRESSED_BLOB_MAGIC_1_0,
};
use crate::DataBlob;

/// File system based chunk store
pub struct ChunkStore {
    name: String, // used for error reporting
    pub(crate) base: PathBuf,
    chunk_dir: PathBuf,
    mutex: Mutex<()>,
    locker: Option<Arc<Mutex<ProcessLocker>>>,
    sync_level: DatastoreFSyncLevel,
}

// TODO: what about sysctl setting vm.vfs_cache_pressure (0 - 100) ?

pub fn verify_chunk_size(size: usize) -> Result<(), Error> {
    static SIZES: [usize; 7] = [
        64 * 1024,
        128 * 1024,
        256 * 1024,
        512 * 1024,
        1024 * 1024,
        2048 * 1024,
        4096 * 1024,
    ];

    if !SIZES.contains(&size) {
        bail!("Got unsupported chunk size '{size}'");
    }
    Ok(())
}

fn digest_to_prefix(digest: &[u8]) -> PathBuf {
    let mut buf = Vec::<u8>::with_capacity(2 + 1 + 2 + 1);

    const HEX_CHARS: &[u8; 16] = b"0123456789abcdef";

    buf.push(HEX_CHARS[(digest[0] as usize) >> 4]);
    buf.push(HEX_CHARS[(digest[0] as usize) & 0xf]);
    buf.push(HEX_CHARS[(digest[1] as usize) >> 4]);
    buf.push(HEX_CHARS[(digest[1] as usize) & 0xf]);
    buf.push(b'/');

    let path = unsafe { String::from_utf8_unchecked(buf) };

    path.into()
}

impl ChunkStore {
    #[doc(hidden)]
    pub unsafe fn panic_store() -> Self {
        Self {
            name: String::new(),
            base: PathBuf::new(),
            chunk_dir: PathBuf::new(),
            mutex: Mutex::new(()),
            locker: None,
            sync_level: Default::default(),
        }
    }

    fn chunk_dir<P: AsRef<Path>>(path: P) -> PathBuf {
        let mut chunk_dir: PathBuf = PathBuf::from(path.as_ref());
        chunk_dir.push(".chunks");

        chunk_dir
    }

    pub fn base(&self) -> &Path {
        &self.base
    }

    pub fn create<P>(
        name: &str,
        path: P,
        uid: nix::unistd::Uid,
        gid: nix::unistd::Gid,
        worker: Option<&dyn WorkerTaskContext>,
        sync_level: DatastoreFSyncLevel,
    ) -> Result<Self, Error>
    where
        P: Into<PathBuf>,
    {
        let base: PathBuf = path.into();

        if !base.is_absolute() {
            bail!("expected absolute path - got {base:?}");
        }

        let chunk_dir = Self::chunk_dir(&base);

        let options = CreateOptions::new().owner(uid).group(gid);

        let default_options = CreateOptions::new();

        match create_path(&base, Some(default_options), Some(options.clone())) {
            Err(err) => bail!("unable to create chunk store '{name}' at {base:?} - {err}"),
            Ok(res) => {
                if !res {
                    nix::unistd::chown(&base, Some(uid), Some(gid))?
                }
            }
        }

        if let Err(err) = create_dir(&chunk_dir, options.clone()) {
            bail!("unable to create chunk store '{name}' subdir {chunk_dir:?} - {err}");
        }

        // create lock file with correct owner/group
        let lockfile_path = Self::lockfile_path(&base);
        proxmox_sys::fs::replace_file(lockfile_path, b"", options.clone(), false)?;

        // create 64*1024 subdirs
        let mut last_percentage = 0;

        for i in 0..64 * 1024 {
            let mut l1path = chunk_dir.clone();
            l1path.push(format!("{:04x}", i));
            if let Err(err) = create_dir(&l1path, options.clone()) {
                bail!(
                    "unable to create chunk store '{}' subdir {:?} - {}",
                    name,
                    l1path,
                    err
                );
            }
            let percentage = (i * 100) / (64 * 1024);
            if percentage != last_percentage {
                if let Some(worker) = worker {
                    task_log!(worker, "Chunkstore create: {}%", percentage)
                }
                last_percentage = percentage;
            }
        }

        Self::open(name, base, sync_level)
    }

    fn lockfile_path<P: Into<PathBuf>>(base: P) -> PathBuf {
        let mut lockfile_path: PathBuf = base.into();
        lockfile_path.push(".lock");
        lockfile_path
    }

    /// Opens the chunk store with a new process locker.
    ///
    /// Note that this must be used with care, as it's dangerous to create two instances on the
    /// same base path, as closing the underlying ProcessLocker drops all locks from this process
    /// on the lockfile (even if separate FDs)
    pub(crate) fn open<P: Into<PathBuf>>(
        name: &str,
        base: P,
        sync_level: DatastoreFSyncLevel,
    ) -> Result<Self, Error> {
        let base: PathBuf = base.into();

        if !base.is_absolute() {
            bail!("expected absolute path - got {:?}", base);
        }

        let chunk_dir = Self::chunk_dir(&base);

        if let Err(err) = std::fs::metadata(&chunk_dir) {
            bail!("unable to open chunk store '{name}' at {chunk_dir:?} - {err}");
        }

        let lockfile_path = Self::lockfile_path(&base);

        let locker = ProcessLocker::new(lockfile_path)?;

        Ok(ChunkStore {
            name: name.to_owned(),
            base,
            chunk_dir,
            locker: Some(locker),
            mutex: Mutex::new(()),
            sync_level,
        })
    }

    pub fn touch_chunk(&self, digest: &[u8; 32]) -> Result<(), Error> {
        // unwrap: only `None` in unit tests
        assert!(self.locker.is_some());

        self.cond_touch_chunk(digest, true)?;
        Ok(())
    }

    pub fn cond_touch_chunk(&self, digest: &[u8; 32], assert_exists: bool) -> Result<bool, Error> {
        // unwrap: only `None` in unit tests
        assert!(self.locker.is_some());

        let (chunk_path, _digest_str) = self.chunk_path(digest);
        self.cond_touch_path(&chunk_path, assert_exists)
    }

    pub fn cond_touch_path(&self, path: &Path, assert_exists: bool) -> Result<bool, Error> {
        // unwrap: only `None` in unit tests
        assert!(self.locker.is_some());

        const UTIME_NOW: i64 = (1 << 30) - 1;
        const UTIME_OMIT: i64 = (1 << 30) - 2;

        let times: [libc::timespec; 2] = [
            // access time -> update to now
            libc::timespec {
                tv_sec: 0,
                tv_nsec: UTIME_NOW,
            },
            // modification time -> keep as is
            libc::timespec {
                tv_sec: 0,
                tv_nsec: UTIME_OMIT,
            },
        ];

        use nix::NixPath;

        let res = path.with_nix_path(|cstr| unsafe {
            let tmp = libc::utimensat(-1, cstr.as_ptr(), &times[0], libc::AT_SYMLINK_NOFOLLOW);
            nix::errno::Errno::result(tmp)
        })?;

        if let Err(err) = res {
            if !assert_exists && err == nix::errno::Errno::ENOENT {
                return Ok(false);
            }
            bail!("update atime failed for chunk/file {path:?} - {err}");
        }

        Ok(true)
    }

    pub fn get_chunk_iterator(
        &self,
    ) -> Result<
        impl Iterator<Item = (Result<proxmox_sys::fs::ReadDirEntry, Error>, usize, bool)>
            + std::iter::FusedIterator,
        Error,
    > {
        // unwrap: only `None` in unit tests
        assert!(self.locker.is_some());

        use nix::dir::Dir;
        use nix::fcntl::OFlag;
        use nix::sys::stat::Mode;

        let base_handle =
            Dir::open(&self.chunk_dir, OFlag::O_RDONLY, Mode::empty()).map_err(|err| {
                format_err!(
                    "unable to open store '{}' chunk dir {:?} - {err}",
                    self.name,
                    self.chunk_dir,
                )
            })?;

        let mut done = false;
        let mut inner: Option<proxmox_sys::fs::ReadDir> = None;
        let mut at = 0;
        let mut percentage = 0;
        Ok(std::iter::from_fn(move || {
            if done {
                return None;
            }

            loop {
                if let Some(ref mut inner) = inner {
                    match inner.next() {
                        Some(Ok(entry)) => {
                            // skip files if they're not a hash
                            let bytes = entry.file_name().to_bytes();
                            if bytes.len() != 64 && bytes.len() != 64 + ".0.bad".len() {
                                continue;
                            }
                            if !bytes.iter().take(64).all(u8::is_ascii_hexdigit) {
                                continue;
                            }

                            let bad = bytes.ends_with(b".bad");
                            return Some((Ok(entry), percentage, bad));
                        }
                        Some(Err(err)) => {
                            // stop after first error
                            done = true;
                            // and pass the error through:
                            return Some((Err(err), percentage, false));
                        }
                        None => (), // open next directory
                    }
                }

                inner = None;

                if at == 0x10000 {
                    done = true;
                    return None;
                }

                let subdir: &str = &format!("{:04x}", at);
                percentage = (at * 100) / 0x10000;
                at += 1;
                match proxmox_sys::fs::read_subdir(base_handle.as_raw_fd(), subdir) {
                    Ok(dir) => {
                        inner = Some(dir);
                        // start reading:
                        continue;
                    }
                    Err(ref err) if err == &nix::errno::Errno::ENOENT => {
                        // non-existing directories are okay, just keep going:
                        continue;
                    }
                    Err(err) => {
                        // other errors are fatal, so end our iteration
                        done = true;
                        // and pass the error through:
                        return Some((
                            Err(format_err!("unable to read subdir '{subdir}' - {err}")),
                            percentage,
                            false,
                        ));
                    }
                }
            }
        })
        .fuse())
    }

    pub fn oldest_writer(&self) -> Option<i64> {
        // unwrap: only `None` in unit tests
        ProcessLocker::oldest_shared_lock(self.locker.clone().unwrap())
    }

    pub fn sweep_unused_chunks(
        &self,
        oldest_writer: i64,
        phase1_start_time: i64,
        status: &mut GarbageCollectionStatus,
        worker: &dyn WorkerTaskContext,
    ) -> Result<(), Error> {
        // unwrap: only `None` in unit tests
        assert!(self.locker.is_some());

        use nix::sys::stat::fstatat;
        use nix::unistd::{unlinkat, UnlinkatFlags};

        let mut min_atime = phase1_start_time - 3600 * 24; // at least 24h (see mount option relatime)

        if oldest_writer < min_atime {
            min_atime = oldest_writer;
        }

        min_atime -= 300; // add 5 mins gap for safety

        let mut last_percentage = 0;
        let mut chunk_count = 0;

        for (entry, percentage, bad) in self.get_chunk_iterator()? {
            if last_percentage != percentage {
                last_percentage = percentage;
                task_log!(worker, "processed {}% ({} chunks)", percentage, chunk_count,);
            }

            worker.check_abort()?;
            worker.fail_on_shutdown()?;

            let (dirfd, entry) = match entry {
                Ok(entry) => (entry.parent_fd(), entry),
                Err(err) => bail!(
                    "chunk iterator on chunk store '{}' failed - {err}",
                    self.name,
                ),
            };

            let filename = entry.file_name();

            let lock = self.mutex.lock();

            if let Ok(stat) = fstatat(dirfd, filename, nix::fcntl::AtFlags::AT_SYMLINK_NOFOLLOW) {
                let file_type = file_type_from_file_stat(&stat);
                if file_type != Some(nix::dir::Type::File) {
                    drop(lock);
                    continue;
                }

                chunk_count += 1;

                if stat.st_atime < min_atime {
                    //let age = now - stat.st_atime;
                    //println!("UNLINK {}  {:?}", age/(3600*24), filename);
                    if let Err(err) = unlinkat(Some(dirfd), filename, UnlinkatFlags::NoRemoveDir) {
                        if bad {
                            status.still_bad += 1;
                        }
                        bail!(
                            "unlinking chunk {filename:?} failed on store '{}' - {err}",
                            self.name,
                        );
                    }
                    if bad {
                        status.removed_bad += 1;
                    } else {
                        status.removed_chunks += 1;
                    }
                    status.removed_bytes += stat.st_size as u64;
                } else if stat.st_atime < oldest_writer {
                    if bad {
                        status.still_bad += 1;
                    } else {
                        status.pending_chunks += 1;
                    }
                    status.pending_bytes += stat.st_size as u64;
                } else {
                    if !bad {
                        status.disk_chunks += 1;
                    }
                    status.disk_bytes += stat.st_size as u64;
                }
            }
            drop(lock);
        }

        Ok(())
    }

    pub fn insert_chunk(&self, chunk: &DataBlob, digest: &[u8; 32]) -> Result<(bool, u64), Error> {
        // unwrap: only `None` in unit tests
        assert!(self.locker.is_some());

        //println!("DIGEST {}", hex::encode(digest));

        let (chunk_path, digest_str) = self.chunk_path(digest);

        let lock = self.mutex.lock();

        let raw_data = chunk.raw_data();
        let encoded_size = raw_data.len() as u64;

        let name = &self.name;

        if let Ok(metadata) = std::fs::metadata(&chunk_path) {
            if !metadata.is_file() {
                bail!("got unexpected file type on store '{name}' for chunk {digest_str}");
            }
            let old_size = metadata.len();
            if encoded_size == old_size {
                self.touch_chunk(digest)?;
                return Ok((true, old_size));
            } else if old_size == 0 {
                log::warn!("found empty chunk '{digest_str}' in store {name}, overwriting");
            } else if chunk.is_encrypted() {
                // incoming chunk is encrypted, possible attack or hash collision!
                let mut existing_file = std::fs::File::open(&chunk_path)?;
                let magic = existing_file.read_exact_allocated(8)?;

                // going from unencrypted to encrypted can never be right, since the digest
                // includes data derived from the encryption key
                if magic == UNCOMPRESSED_BLOB_MAGIC_1_0 || magic == COMPRESSED_BLOB_MAGIC_1_0 {
                    bail!("Overwriting unencrypted chunk '{digest_str}' on store '{name}' with encrypted chunk with same digest not allowed!");
                }

                // if both chunks are uncompressed and encrypted and have the same digest, but
                // their sizes are different, one of them *must* be invalid
                if magic == ENCRYPTED_BLOB_MAGIC_1_0 && !chunk.is_compressed() {
                    bail!("Overwriting existing (encrypted) chunk '{digest_str}' on store '{name}' is not allowed!")
                }

                // we can't tell if either chunk is invalid if either or both of them are
                // compressed, the size mismatch could be caused by different zstd versions
                // so let's keep the one that was uploaded first, bit-rot is hopefully detected by
                // verification at some point..
                self.touch_chunk(digest)?;
                return Ok((true, old_size));
            } else if old_size < encoded_size {
                log::debug!("Got another copy of chunk with digest '{digest_str}', existing chunk is smaller, discarding uploaded one.");
                self.touch_chunk(digest)?;
                return Ok((true, old_size));
            } else {
                log::debug!("Got another copy of chunk with digest '{digest_str}', existing chunk is bigger, replacing with uploaded one.");
            }
        }

        let chunk_dir_path = chunk_path
            .parent()
            .ok_or_else(|| format_err!("unable to get chunk dir"))?;

        proxmox_sys::fs::replace_file(
            &chunk_path,
            raw_data,
            CreateOptions::new(),
            self.sync_level == DatastoreFSyncLevel::File,
        )
        .map_err(|err| {
            format_err!("inserting chunk on store '{name}' failed for {digest_str} - {err}")
        })?;

        if self.sync_level == DatastoreFSyncLevel::File {
            // fsync dir handle to persist the tmp rename
            let dir = std::fs::File::open(chunk_dir_path)?;
            nix::unistd::fsync(dir.as_raw_fd())
                .map_err(|err| format_err!("fsync failed: {err}"))?;
        }

        drop(lock);

        Ok((false, encoded_size))
    }

    pub fn chunk_path(&self, digest: &[u8; 32]) -> (PathBuf, String) {
        // unwrap: only `None` in unit tests
        assert!(self.locker.is_some());

        let mut chunk_path = self.chunk_dir.clone();
        let prefix = digest_to_prefix(digest);
        chunk_path.push(&prefix);
        let digest_str = hex::encode(digest);
        chunk_path.push(&digest_str);
        (chunk_path, digest_str)
    }

    pub fn relative_path(&self, path: &Path) -> PathBuf {
        // unwrap: only `None` in unit tests
        assert!(self.locker.is_some());

        let mut full_path = self.base.clone();
        full_path.push(path);
        full_path
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn base_path(&self) -> PathBuf {
        // unwrap: only `None` in unit tests
        assert!(self.locker.is_some());

        self.base.clone()
    }

    pub fn try_shared_lock(&self) -> Result<ProcessLockSharedGuard, Error> {
        // unwrap: only `None` in unit tests
        ProcessLocker::try_shared_lock(self.locker.clone().unwrap())
    }

    pub fn try_exclusive_lock(&self) -> Result<ProcessLockExclusiveGuard, Error> {
        // unwrap: only `None` in unit tests
        ProcessLocker::try_exclusive_lock(self.locker.clone().unwrap())
    }
}

#[test]
fn test_chunk_store1() {
    let mut path = std::fs::canonicalize(".").unwrap(); // we need absolute path
    path.push(".testdir");

    if let Err(_e) = std::fs::remove_dir_all(".testdir") { /* ignore */ }

    let chunk_store = ChunkStore::open("test", &path, DatastoreFSyncLevel::None);
    assert!(chunk_store.is_err());

    let user = nix::unistd::User::from_uid(nix::unistd::Uid::current())
        .unwrap()
        .unwrap();
    let chunk_store = ChunkStore::create(
        "test",
        &path,
        user.uid,
        user.gid,
        None,
        DatastoreFSyncLevel::None,
    )
    .unwrap();

    let (chunk, digest) = crate::data_blob::DataChunkBuilder::new(&[0u8, 1u8])
        .build()
        .unwrap();

    let (exists, _) = chunk_store.insert_chunk(&chunk, &digest).unwrap();
    assert!(!exists);

    let (exists, _) = chunk_store.insert_chunk(&chunk, &digest).unwrap();
    assert!(exists);

    let chunk_store = ChunkStore::create(
        "test",
        &path,
        user.uid,
        user.gid,
        None,
        DatastoreFSyncLevel::None,
    );
    assert!(chunk_store.is_err());

    if let Err(_e) = std::fs::remove_dir_all(".testdir") { /* ignore */ }
}
