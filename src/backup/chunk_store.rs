use failure::*;
use std::path::{Path, PathBuf};
use std::io::Write;
use std::time::Duration;

use openssl::sha;
use std::sync::Mutex;

use std::fs::File;
use std::os::unix::io::{AsRawFd, RawFd};

use crate::tools;
use crate::tools::borrow::Tied;

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

pub struct ChunkStore {
    name: String, // used for error reporting
    pub (crate) base: PathBuf,
    chunk_dir: PathBuf,
    mutex: Mutex<bool>,
    _lockfile: File,
}

// TODO: what about sysctl setting vm.vfs_cache_pressure (0 - 100) ?

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

// This is one thing which would actually get nicer with futures & tokio-fs...
pub struct ChunkIterator {
    base_dir: nix::dir::Dir,
    index: usize,
    subdir: Option<
        Tied<nix::dir::Dir, Iterator<Item = nix::Result<nix::dir::Entry>>>
        >,
    subdir_fd: RawFd,
    progress: Option<fn(u8)>,
}

impl ChunkIterator {
    fn new(base_dir: nix::dir::Dir) -> Self {
        ChunkIterator {
            base_dir,
            index: 0,
            subdir: None,
            subdir_fd: 0,
            progress: None,
        }
    }

    fn with_progress(base_dir: nix::dir::Dir, progress: fn(u8)) -> Self {
        let mut me = Self::new(base_dir);
        me.progress = Some(progress);
        me
    }

    fn next_subdir(&mut self) -> Result<bool, Error> {
        if self.index == 0x10000 {
            return Ok(false);
        }

        let l1name = PathBuf::from(format!("{:04x}", self.index));
        self.index += 1;
        if let Some(cb) = self.progress {
            let prev = ((self.index-1) * 100) / 0x10000;
            let now = (self.index * 100) / 0x10000;
            if prev != now {
                cb(now as u8);
            }
        }

        use nix::dir::{Dir, Entry};
        use nix::fcntl::OFlag;
        use nix::sys::stat::Mode;
        match Dir::openat(self.base_dir.as_raw_fd(), &l1name, OFlag::O_RDONLY, Mode::empty()) {
            Ok(dir) => {
                self.subdir_fd = dir.as_raw_fd();
                self.subdir = Some(Tied::new(dir, |dir| {
                    Box::new(unsafe { (*dir).iter() })
                    as Box<Iterator<Item = nix::Result<Entry>>>
                }));
                return Ok(true);
            }
            Err(err) => {
                self.index = 0x10000;
                bail!("unable to open chunk dir {:?}: {}", l1name, err);
            }
        }
    }
}

impl Iterator for ChunkIterator {
    type Item = Result<(RawFd, nix::dir::Entry), Error>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.subdir {
                None => {
                    match self.next_subdir() {
                        Ok(true) => continue, // Enter the Some case
                        Ok(false) => return None,
                        Err(e) => return Some(Err(e)),
                    }
                }
                Some(ref mut dir) => {
                    let dir = dir.as_mut();
                    match dir.next() {
                        Some(Ok(entry)) => return Some(Ok((self.subdir_fd, entry))),
                        Some(Err(e)) => return Some(Err(e.into())),
                        None => {
                            // Go to the next directory
                            self.subdir = None;
                            continue;
                        }
                    }
                }
            }
        }
    }
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

        // make sure only one process/thread/task can use it
        let lockfile = tools::open_file_locked(
            lockfile_path, Duration::from_secs(10))?;

        Ok(ChunkStore {
            name: name.to_owned(),
            base,
            chunk_dir,
            _lockfile: lockfile,
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

        let mut f = std::fs::File::open(&chunk_path)?;

        let stat = nix::sys::stat::fstat(f.as_raw_fd())?;
        let size = stat.st_size as usize;

        if buffer.capacity() < size {
            let mut newsize =  buffer.capacity();
            while newsize < size { newsize = newsize << 1; }
            let additional = newsize - buffer.len();
            buffer.reserve_exact(additional);
        }
        unsafe { buffer.set_len(size); }

        use std::io::Read;

        f.read_exact(buffer.as_mut_slice())?;

        Ok(())
    }

    fn sweep_old_files(&self, handle: &mut nix::dir::Dir, status: &mut GarbageCollectionStatus) -> Result<(), Error> {

        let rawfd = handle.as_raw_fd();

        let now = unsafe { libc::time(std::ptr::null_mut()) };

        for entry in handle.iter() {
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => continue /* ignore */,
            };
            let file_type = match entry.file_type() {
                Some(file_type) => file_type,
                None => bail!("unsupported file system type on chunk store '{}'", self.name),
            };
            if file_type != nix::dir::Type::File { continue; }

            let filename = entry.file_name();
            if let Ok(stat) = nix::sys::stat::fstatat(rawfd, filename, nix::fcntl::AtFlags::AT_SYMLINK_NOFOLLOW) {
                let age = now - stat.st_atime;
                //println!("FOUND {}  {:?}", age/(3600*24), filename);
                if age/(3600*24) >= 2 {
                    println!("UNLINK {}  {:?}", age/(3600*24), filename);
                    let res = unsafe { libc::unlinkat(rawfd, filename.as_ptr(), 0) };
                    if res != 0 {
                        let err = nix::Error::last();
                        bail!("unlink chunk {:?} failed on store '{}' - {}", filename, self.name, err);
                    }
                } else {
                    status.disk_chunks += 1;
                    status.disk_bytes += stat.st_size as usize;

                }
            }
        }
        Ok(())
    }

    pub fn sweep_unused_chunks(&self, status: &mut GarbageCollectionStatus) -> Result<(), Error> {

        use nix::fcntl::OFlag;
        use nix::sys::stat::Mode;
        use nix::dir::Dir;

        let base_handle = match Dir::open(
            &self.chunk_dir, OFlag::O_RDONLY, Mode::empty()) {
            Ok(h) => h,
            Err(err) => bail!("unable to open store '{}' chunk dir {:?} - {}",
                              self.name, self.chunk_dir, err),
        };

        let base_fd = base_handle.as_raw_fd();

        let mut last_percentage = 0;

        for i in 0..64*1024 {

            let percentage = (i*100)/(64*1024);
            if percentage != last_percentage {
                eprintln!("Percentage done: {}", percentage);
                last_percentage = percentage;
            }

            let l1name = PathBuf::from(format!("{:04x}", i));
            match nix::dir::Dir::openat(base_fd, &l1name, OFlag::O_RDONLY, Mode::empty()) {
                Ok(mut h) => {
                    //println!("SCAN {:?} {:?}", l1name);
                   self.sweep_old_files(&mut h, status)?;
                }
                Err(err) => bail!("unable to open store '{}' dir {:?}/{:?} - {}",
                                  self.name, self.chunk_dir, l1name, err),
            };
        }
        Ok(())
    }

    pub fn insert_chunk(&self, chunk: &[u8]) -> Result<(bool, [u8; 32]), Error> {

        // fixme: use Sha512/256 when available
        let mut hasher = sha::Sha256::new();
        hasher.update(chunk);

        let digest = hasher.finish();

        //println!("DIGEST {}", tools::digest_to_hex(&digest));

        let mut chunk_path = self.chunk_dir.clone();
        let prefix = digest_to_prefix(&digest);
        chunk_path.push(&prefix);
        let digest_str = tools::digest_to_hex(&digest);
        chunk_path.push(&digest_str);

        let lock = self.mutex.lock();

        if let Ok(metadata) = std::fs::metadata(&chunk_path) {
            if metadata.is_file() {
                 return Ok((true, digest));
            } else {
                bail!("Got unexpected file type on store '{}' for chunk {}", self.name, digest_str);
            }
        }

        let mut tmp_path = chunk_path.clone();
        tmp_path.set_extension("tmp");
        let mut f = std::fs::File::create(&tmp_path)?;
        f.write_all(chunk)?;

        if let Err(err) = std::fs::rename(&tmp_path, &chunk_path) {
            if let Err(_) = std::fs::remove_file(&tmp_path)  { /* ignore */ }
            bail!("Atomic rename on store '{}' failed for chunk {} - {}", self.name, digest_str, err);
        }

        //println!("PATH {:?}", chunk_path);

        drop(lock);

        Ok((false, digest))
    }

    pub fn relative_path(&self, path: &Path) -> PathBuf {

        let mut full_path = self.base.clone();
        full_path.push(path);
        full_path
    }

    pub fn base_path(&self) -> PathBuf {
        self.base.clone()
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
    let (exists, _) = chunk_store.insert_chunk(&[0u8, 1u8]).unwrap();
    assert!(!exists);

    let (exists, _) = chunk_store.insert_chunk(&[0u8, 1u8]).unwrap();
    assert!(exists);


    let chunk_store = ChunkStore::create("test", &path);
    assert!(chunk_store.is_err());

    if let Err(_e) = std::fs::remove_dir_all(".testdir") { /* ignore */ }
}
