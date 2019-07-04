use failure::*;

use std::path::{Path, PathBuf};
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::os::unix::io::AsRawFd;
use serde::Serialize;

use crate::tools;
use super::DataChunk;
use crate::server::WorkerTask;

#[derive(Clone, Serialize)]
pub struct GarbageCollectionStatus {
    pub upid: Option<String>,
    pub index_file_count: usize,
    pub index_data_bytes: u64,
    pub disk_bytes: u64,
    pub disk_chunks: usize,
    pub removed_bytes: u64,
    pub removed_chunks: usize,
}

impl Default for GarbageCollectionStatus {
    fn default() -> Self {
        GarbageCollectionStatus {
            upid: None,
            index_file_count: 0,
            index_data_bytes: 0,
            disk_bytes: 0,
            disk_chunks: 0,
            removed_bytes: 0,
            removed_chunks: 0,
        }
    }
}

/// File system based chunk store
pub struct ChunkStore {
    name: String, // used for error reporting
    pub (crate) base: PathBuf,
    chunk_dir: PathBuf,
    mutex: Mutex<bool>,
    locker: Arc<Mutex<tools::ProcessLocker>>,
}

// TODO: what about sysctl setting vm.vfs_cache_pressure (0 - 100) ?

pub fn verify_chunk_size(size: usize) -> Result<(), Error> {

    static SIZES: [usize; 7] = [64*1024, 128*1024, 256*1024, 512*1024, 1024*1024, 2048*1024, 4096*1024];

    if !SIZES.contains(&size) {
        bail!("Got unsupported chunk size '{}'", size);
    }
    Ok(())
}

fn digest_to_prefix(digest: &[u8]) -> PathBuf {

    let mut buf = Vec::<u8>::with_capacity(2+1+2+1);

    const HEX_CHARS: &'static [u8; 16] = b"0123456789abcdef";

    buf.push(HEX_CHARS[(digest[0] as usize) >> 4]);
    buf.push(HEX_CHARS[(digest[0] as usize) &0xf]);
    buf.push(HEX_CHARS[(digest[1] as usize) >> 4]);
    buf.push(HEX_CHARS[(digest[1] as usize) & 0xf]);
    buf.push('/' as u8);

    let path = unsafe { String::from_utf8_unchecked(buf)};

    path.into()
}

impl ChunkStore {

    fn chunk_dir<P: AsRef<Path>>(path: P) -> PathBuf {

        let mut chunk_dir: PathBuf = PathBuf::from(path.as_ref());
        chunk_dir.push(".chunks");

        chunk_dir
    }

    pub fn create<P: Into<PathBuf>>(name: &str, path: P) -> Result<Self, Error> {

        let base: PathBuf = path.into();

        if !base.is_absolute() {
            bail!("expected absolute path - got {:?}", base);
        }

        let chunk_dir = Self::chunk_dir(&base);

        if let Err(err) = std::fs::create_dir(&base) {
            bail!("unable to create chunk store '{}' at {:?} - {}", name, base, err);
        }

        if let Err(err) = std::fs::create_dir(&chunk_dir) {
            bail!("unable to create chunk store '{}' subdir {:?} - {}", name, chunk_dir, err);
        }

        // create 64*1024 subdirs
        let mut last_percentage = 0;

        for i in 0..64*1024 {
            let mut l1path = chunk_dir.clone();
            l1path.push(format!("{:04x}", i));
            if let Err(err) = std::fs::create_dir(&l1path) {
                bail!("unable to create chunk store '{}' subdir {:?} - {}", name, l1path, err);
            }
            let percentage = (i*100)/(64*1024);
            if percentage != last_percentage {
                eprintln!("Percentage done: {}", percentage);
                last_percentage = percentage;
            }
        }

        Self::open(name, base)
    }

    pub fn open<P: Into<PathBuf>>(name: &str, path: P) -> Result<Self, Error> {

        let base: PathBuf = path.into();

        if !base.is_absolute() {
            bail!("expected absolute path - got {:?}", base);
        }

        let chunk_dir = Self::chunk_dir(&base);

        if let Err(err) = std::fs::metadata(&chunk_dir) {
            bail!("unable to open chunk store '{}' at {:?} - {}", name, chunk_dir, err);
        }

        let mut lockfile_path = base.clone();
        lockfile_path.push(".lock");

        let locker = tools::ProcessLocker::new(&lockfile_path)?;

        Ok(ChunkStore {
            name: name.to_owned(),
            base,
            chunk_dir,
            locker,
            mutex: Mutex::new(false)
        })
    }

    pub fn touch_chunk(&self, digest: &[u8; 32]) -> Result<(), Error> {

        let (chunk_path, _digest_str) = self.chunk_path(digest);

        const UTIME_NOW: i64 = ((1 << 30) - 1);
        const UTIME_OMIT: i64 = ((1 << 30) - 2);

        let times: [libc::timespec; 2] = [
            libc::timespec { tv_sec: 0, tv_nsec: UTIME_NOW },
            libc::timespec { tv_sec: 0, tv_nsec: UTIME_OMIT }
        ];

        use nix::NixPath;

        let res = chunk_path.with_nix_path(|cstr| unsafe {
            libc::utimensat(-1, cstr.as_ptr(), &times[0], libc::AT_SYMLINK_NOFOLLOW)
        })?;

        if let Err(err) = nix::errno::Errno::result(res) {
            bail!("updata atime failed for chunk {:?} - {}", chunk_path, err);
        }

        Ok(())
    }

    pub fn read_chunk(&self, digest: &[u8; 32]) -> Result<DataChunk, Error> {

        let (chunk_path, digest_str) = self.chunk_path(digest);
        let mut file = std::fs::File::open(&chunk_path)
            .map_err(|err| {
                format_err!(
                    "store '{}', unable to read chunk '{}' - {}",
                    self.name,
                    digest_str,
                    err,
                )
            })?;

        DataChunk::load(&mut file, *digest)
    }

    pub fn get_chunk_iterator(
        &self,
    ) -> Result<
        impl Iterator<Item = (Result<tools::fs::ReadDirEntry, Error>, usize)> + std::iter::FusedIterator,
        Error
    > {
        use nix::dir::Dir;
        use nix::fcntl::OFlag;
        use nix::sys::stat::Mode;

        let base_handle = Dir::open(&self.chunk_dir, OFlag::O_RDONLY, Mode::empty())
            .map_err(|err| {
                format_err!(
                    "unable to open store '{}' chunk dir {:?} - {}",
                    self.name,
                    self.chunk_dir,
                    err,
                )
            })?;

        let mut done = false;
        let mut inner: Option<tools::fs::ReadDir> = None;
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
                            if bytes.len() != 64 {
                                continue;
                            }
                            if !bytes.iter().all(u8::is_ascii_hexdigit) {
                                continue;
                            }
                            return Some((Ok(entry), percentage));
                        }
                        Some(Err(err)) => {
                            // stop after first error
                            done = true;
                            // and pass the error through:
                            return Some((Err(err), percentage));
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
                match tools::fs::read_subdir(base_handle.as_raw_fd(), subdir) {
                    Ok(dir) => {
                        inner = Some(dir);
                        // start reading:
                        continue;
                    }
                    Err(ref err) if err.as_errno() == Some(nix::errno::Errno::ENOENT) => {
                        // non-existing directories are okay, just keep going:
                        continue;
                    }
                    Err(err) => {
                        // other errors are fatal, so end our iteration
                        done = true;
                        // and pass the error through:
                        return Some((Err(format_err!("unable to read subdir '{}' - {}", subdir, err)), percentage));
                    }
                }
            }
        }).fuse())
    }

    pub fn oldest_writer(&self) -> Option<i64> {
        tools::ProcessLocker::oldest_shared_lock(self.locker.clone())
    }

    pub fn sweep_unused_chunks(
        &self,
        oldest_writer: Option<i64>,
        status: &mut GarbageCollectionStatus,
        worker: Arc<WorkerTask>,
    ) -> Result<(), Error> {
        use nix::sys::stat::fstatat;

        let now = unsafe { libc::time(std::ptr::null_mut()) };

        let mut min_atime = now - 3600*24; // at least 24h (see mount option relatime)

        if let Some(stamp) = oldest_writer {
            if stamp < min_atime {
                min_atime = stamp;
            }
        }

        min_atime -= 300; // add 5 mins gap for safety

        let mut last_percentage = 0;
        let mut chunk_count = 0;

        for (entry, percentage) in self.get_chunk_iterator()? {
            if last_percentage != percentage {
                last_percentage = percentage;
                worker.log(format!("percentage done: {}, chunk count: {}", percentage, chunk_count));
            }

            tools::fail_on_shutdown()?;

            let (dirfd, entry) = match entry {
                Ok(entry) => (entry.parent_fd(), entry),
                Err(err) => bail!("chunk iterator on chunk store '{}' failed - {}", self.name, err),
            };

            let file_type = match entry.file_type() {
                Some(file_type) => file_type,
                None => bail!("unsupported file system type on chunk store '{}'", self.name),
            };
            if file_type != nix::dir::Type::File {
                continue;
            }

            chunk_count += 1;

            let filename = entry.file_name();

            let lock = self.mutex.lock();

            if let Ok(stat) = fstatat(dirfd, filename, nix::fcntl::AtFlags::AT_SYMLINK_NOFOLLOW) {
                let age = now - stat.st_atime;
                //println!("FOUND {}  {:?}", age/(3600*24), filename);
                if stat.st_atime < min_atime {
                    println!("UNLINK {}  {:?}", age/(3600*24), filename);
                    let res = unsafe { libc::unlinkat(dirfd, filename.as_ptr(), 0) };
                    if res != 0 {
                        let err = nix::Error::last();
                        bail!(
                            "unlink chunk {:?} failed on store '{}' - {}",
                            filename,
                            self.name,
                            err,
                        );
                    }
                    status.removed_chunks += 1;
                    status.removed_bytes += stat.st_size as u64;
               } else {
                    status.disk_chunks += 1;
                    status.disk_bytes += stat.st_size as u64;
                }
            }
            drop(lock);
        }

        Ok(())
    }

    pub fn insert_chunk(
        &self,
        chunk: &DataChunk,
    ) -> Result<(bool, u64), Error> {

        let digest = chunk.digest();

        //println!("DIGEST {}", proxmox::tools::digest_to_hex(digest));

        let (chunk_path, digest_str) = self.chunk_path(digest);

        let lock = self.mutex.lock();

        if let Ok(metadata) = std::fs::metadata(&chunk_path) {
            if metadata.is_file() {
                return Ok((true, metadata.len()));
            } else {
                bail!("Got unexpected file type on store '{}' for chunk {}", self.name, digest_str);
            }
        }

        let mut tmp_path = chunk_path.clone();
        tmp_path.set_extension("tmp");

        let mut file = std::fs::File::create(&tmp_path)?;

        let raw_data = chunk.raw_data();
        let encoded_size = raw_data.len() as u64;

        file.write_all(raw_data)?;

        if let Err(err) = std::fs::rename(&tmp_path, &chunk_path) {
            if let Err(_) = std::fs::remove_file(&tmp_path)  { /* ignore */ }
            bail!(
                "Atomic rename on store '{}' failed for chunk {} - {}",
                self.name,
                digest_str,
                err,
            );
        }

        drop(lock);

        Ok((false, encoded_size))
    }

    pub fn chunk_path(&self, digest:&[u8; 32]) -> (PathBuf, String) {
        let mut chunk_path = self.chunk_dir.clone();
        let prefix = digest_to_prefix(digest);
        chunk_path.push(&prefix);
        let digest_str = proxmox::tools::digest_to_hex(digest);
        chunk_path.push(&digest_str);
        (chunk_path, digest_str)
    }

    pub fn relative_path(&self, path: &Path) -> PathBuf {

        let mut full_path = self.base.clone();
        full_path.push(path);
        full_path
    }

    pub fn base_path(&self) -> PathBuf {
        self.base.clone()
    }

    pub fn try_shared_lock(&self) -> Result<tools::ProcessLockSharedGuard, Error> {
        tools::ProcessLocker::try_shared_lock(self.locker.clone())
    }

    pub fn try_exclusive_lock(&self) -> Result<tools::ProcessLockExclusiveGuard, Error> {
        tools::ProcessLocker::try_exclusive_lock(self.locker.clone())
    }
}


#[test]
fn test_chunk_store1() {

    let mut path = std::fs::canonicalize(".").unwrap(); // we need absulute path
    path.push(".testdir");

    if let Err(_e) = std::fs::remove_dir_all(".testdir") { /* ignore */ }

    let chunk_store = ChunkStore::open("test", &path);
    assert!(chunk_store.is_err());

    let chunk_store = ChunkStore::create("test", &path).unwrap();

    let chunk = super::DataChunkBuilder::new(&[0u8, 1u8]).build().unwrap();

    let (exists, _) = chunk_store.insert_chunk(&chunk).unwrap();
    assert!(!exists);

    let (exists, _) = chunk_store.insert_chunk(&chunk).unwrap();
    assert!(exists);


    let chunk_store = ChunkStore::create("test", &path);
    assert!(chunk_store.is_err());

    if let Err(_e) = std::fs::remove_dir_all(".testdir") { /* ignore */ }
}
