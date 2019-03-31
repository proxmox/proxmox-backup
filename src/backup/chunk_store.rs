use failure::*;

use std::path::{Path, PathBuf};
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::os::unix::io::AsRawFd;

use openssl::sha;

use crate::tools;

pub struct GarbageCollectionStatus {
    pub used_bytes: usize,
    pub used_chunks: usize,
    pub disk_bytes: usize,
    pub disk_chunks: usize,
}

impl Default for GarbageCollectionStatus {
    fn default() -> Self {
        GarbageCollectionStatus {
            used_bytes: 0,
            used_chunks: 0,
            disk_bytes: 0,
            disk_chunks: 0,
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

pub fn verify_chunk_size(size: u64) -> Result<(), Error> {

    static SIZES: [u64; 7] = [64*1024, 128*1024, 256*1024, 512*1024, 1024*1024, 2048*1024, 4096*1024];

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

    pub fn touch_chunk(&self, digest:&[u8]) -> Result<(), Error> {

        let mut chunk_path = self.chunk_dir.clone();
        let prefix = digest_to_prefix(&digest);
        chunk_path.push(&prefix);
        let digest_str = tools::digest_to_hex(&digest);
        chunk_path.push(&digest_str);

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

    pub fn read_chunk(&self, digest:&[u8], buffer: &mut Vec<u8>) -> Result<(), Error> {

        let mut chunk_path = self.chunk_dir.clone();
        let prefix = digest_to_prefix(&digest);
        chunk_path.push(&prefix);
        let digest_str = tools::digest_to_hex(&digest);
        chunk_path.push(&digest_str);

        buffer.clear();
        let f = std::fs::File::open(&chunk_path)?;
        let mut decoder = zstd::stream::Decoder::new(f)?;

        decoder.read_to_end(buffer)?;

        Ok(())
    }

    pub fn get_chunk_iterator(
        &self,
        print_percentage: bool,
    ) -> Result<
        impl Iterator<Item = Result<tools::fs::ReadDirEntry, Error>> + std::iter::FusedIterator,
        Error
    > {
        use nix::dir::Dir;
        use nix::fcntl::OFlag;
        use nix::sys::stat::Mode;

        let base_handle = match Dir::open(
            &self.chunk_dir, OFlag::O_RDONLY, Mode::empty()) {
            Ok(h) => h,
            Err(err) => bail!("unable to open store '{}' chunk dir {:?} - {}",
                              self.name, self.chunk_dir, err),
        };

        let mut verbose = true;
        let mut last_percentage = 0;

        Ok((0..0x10000).filter_map(move |index| {
            if print_percentage {
                let percentage = (index * 100) / 0x10000;
                if last_percentage != percentage {
                    last_percentage = percentage;
                    eprintln!("percentage done: {}", percentage);
                }
            }
            let subdir: &str = &format!("{:04x}", index);
            match tools::fs::read_subdir(base_handle.as_raw_fd(), subdir) {
                Err(e) => {
                    if verbose {
                        eprintln!("Error iterating through chunks: {}", e);
                        verbose = false;
                    }
                    None
                }
                Ok(iter) => Some(iter),
            }
        })
        .flatten()
        .filter(|entry| {
            // Check that the file name is actually a hash! (64 hex digits)
            let entry = match entry {
                Err(_) => return true, // pass errors onwards
                Ok(ref entry) => entry,
            };
            let bytes = entry.file_name().to_bytes();
            if bytes.len() != 64 {
                return false;
            }
            for b in bytes {
                if !b.is_ascii_hexdigit() {
                    return false;
                }
            }
            true
        }))
    }

    pub fn sweep_unused_chunks(&self, status: &mut GarbageCollectionStatus) -> Result<(), Error> {
        use nix::sys::stat::fstatat;

        let now = unsafe { libc::time(std::ptr::null_mut()) };

        for entry in self.get_chunk_iterator(true)? {
            let (dirfd, entry) = match entry {
                Ok(entry) => (entry.parent_fd(), entry),
                Err(_) => continue, // ignore errors
            };

            let file_type = match entry.file_type() {
                Some(file_type) => file_type,
                None => bail!("unsupported file system type on chunk store '{}'", self.name),
            };
            if file_type != nix::dir::Type::File {
                continue;
            }

            let filename = entry.file_name();

            let lock = self.mutex.lock();

            if let Ok(stat) = fstatat(dirfd, filename, nix::fcntl::AtFlags::AT_SYMLINK_NOFOLLOW) {
                let age = now - stat.st_atime;
                //println!("FOUND {}  {:?}", age/(3600*24), filename);
                if age/(3600*24) >= 2 {
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
                } else {
                    status.disk_chunks += 1;
                    status.disk_bytes += stat.st_size as usize;
                }
            }
            drop(lock);
        }
        Ok(())
    }

    pub fn insert_chunk(&self, chunk: &[u8]) -> Result<(bool, [u8; 32], u64), Error> {

        // fixme: use Sha512/256 when available
        let digest = sha::sha256(chunk);
        let (new, csize) = self.insert_chunk_noverify(&digest, chunk)?;
        Ok((new, digest, csize))
    }

    pub fn insert_chunk_noverify(
        &self,
        digest: &[u8; 32],
        chunk: &[u8],
    ) -> Result<(bool, u64), Error> {

        //println!("DIGEST {}", tools::digest_to_hex(&digest));

        let mut chunk_path = self.chunk_dir.clone();
        let prefix = digest_to_prefix(digest);
        chunk_path.push(&prefix);
        let digest_str = tools::digest_to_hex(digest);
        chunk_path.push(&digest_str);

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

        let f = std::fs::File::create(&tmp_path)?;

        let mut encoder = zstd::stream::Encoder::new(f, 1)?;

        encoder.write_all(chunk)?;
        let f = encoder.finish()?;

        if let Err(err) = std::fs::rename(&tmp_path, &chunk_path) {
            if let Err(_) = std::fs::remove_file(&tmp_path)  { /* ignore */ }
            bail!(
                "Atomic rename on store '{}' failed for chunk {} - {}",
                self.name,
                digest_str,
                err,
            );
        }

        // fixme: is there a better way to get the compressed size?
        let stat = nix::sys::stat::fstat(f.as_raw_fd())?;
        let compressed_size = stat.st_size as u64;

        //println!("PATH {:?}", chunk_path);

        drop(lock);

        Ok((false, compressed_size))
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
    let (exists, _, _) = chunk_store.insert_chunk(&[0u8, 1u8]).unwrap();
    assert!(!exists);

    let (exists, _, _) = chunk_store.insert_chunk(&[0u8, 1u8]).unwrap();
    assert!(exists);


    let chunk_store = ChunkStore::create("test", &path);
    assert!(chunk_store.is_err());

    if let Err(_e) = std::fs::remove_dir_all(".testdir") { /* ignore */ }
}
