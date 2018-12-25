use failure::*;
use std::path::{Path, PathBuf};
use std::io::Write;
use std::time::Duration;

use openssl::sha;
use std::sync::Mutex;

use std::fs::File;
use std::os::unix::io::AsRawFd;

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

pub struct ChunkStore {
    name: String, // used for error reporting
    pub (crate) base: PathBuf,
    chunk_dir: PathBuf,
    mutex: Mutex<bool>,
    _lockfile: File,
}

const HEX_CHARS: &'static [u8; 16] = b"0123456789abcdef";

// TODO: what about sysctl setting vm.vfs_cache_pressure (0 - 100) ?

pub fn digest_to_hex(digest: &[u8]) -> String {

    let mut buf = Vec::<u8>::with_capacity(digest.len()*2);

    for i in 0..digest.len() {
        buf.push(HEX_CHARS[(digest[i] >> 4) as usize]);
        buf.push(HEX_CHARS[(digest[i] & 0xf) as usize]);
    }

    unsafe { String::from_utf8_unchecked(buf) }
}

fn digest_to_prefix(digest: &[u8]) -> PathBuf {

    let mut buf = Vec::<u8>::with_capacity(2+1+2+1);

    buf.push(HEX_CHARS[(digest[0] as usize) >> 4]);
    buf.push(HEX_CHARS[(digest[0] as usize) &0xf]);
    buf.push('/' as u8);

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
        let chunk_dir = Self::chunk_dir(&base);

        if let Err(err) = std::fs::create_dir(&base) {
            bail!("unable to create chunk store '{}' at {:?} - {}", name, base, err);
        }

        if let Err(err) = std::fs::create_dir(&chunk_dir) {
            bail!("unable to create chunk store '{}' subdir {:?} - {}", name, chunk_dir, err);
        }

        // create 256*256 subdirs
        let mut last_percentage = 0;

        for i in 0..256 {
            let mut l1path = chunk_dir.clone();
            l1path.push(format!("{:02x}",i));
            if let Err(err) = std::fs::create_dir(&l1path) {
                bail!("unable to create chunk store '{}' subdir {:?} - {}", name, l1path, err);
            }
            for j in 0..256 {
                let mut l2path = l1path.clone();
                l2path.push(format!("{:02x}",j));
                if let Err(err) = std::fs::create_dir(&l2path) {
                    bail!("unable to create chunk store '{}' subdir {:?} - {}", name, l2path, err);
                }
                let percentage = ((i*256+j)*100)/(256*256);
                if percentage != last_percentage {
                    eprintln!("Percentage done: {}", percentage);
                    last_percentage = percentage;
                }
            }
        }

        Self::open(name, base)
    }

    pub fn open<P: Into<PathBuf>>(name: &str, path: P) -> Result<Self, Error> {

        let base: PathBuf = path.into();
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

    pub fn touch_chunk(&self, digest:&[u8]) ->  Result<(), Error> {

         let mut chunk_path = self.chunk_dir.clone();
        let prefix = digest_to_prefix(&digest);
        chunk_path.push(&prefix);
        let digest_str = digest_to_hex(&digest);
        chunk_path.push(&digest_str);

        const UTIME_NOW: i64 = ((1 << 30) - 1);
        const UTIME_OMIT: i64 = ((1 << 30) - 2);

        let mut times: [libc::timespec; 2] = [
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

    pub fn sweep_used_chunks(&self, status: &mut GarbageCollectionStatus) -> Result<(), Error> {

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

        for i in 0..256 {
            let l1name = PathBuf::from(format!("{:02x}", i));
            let mut l1_handle = match nix::dir::Dir::openat(
                base_fd, &l1name, OFlag::O_RDONLY, Mode::empty()) {
                Ok(h) => h,
                Err(err) => bail!("unable to open store '{}' dir {:?}/{:?} - {}",
                                  self.name, self.chunk_dir, l1name, err),
            };

            let l1_fd = l1_handle.as_raw_fd();

            for j in 0..256 {
                let l2name = PathBuf::from(format!("{:02x}", j));

                let percentage = ((i*256+j)*100)/(256*256);
                if percentage != last_percentage {
                    eprintln!("Percentage done: {}", percentage);
                    last_percentage = percentage;
                }
                //println!("SCAN {:?} {:?}", l1name, l2name);

                let mut l2_handle = match Dir::openat(
                    l1_fd, &l2name, OFlag::O_RDONLY, Mode::empty()) {
                    Ok(h) => h,
                    Err(err) => bail!(
                        "unable to open store '{}' dir {:?}/{:?}/{:?} - {}",
                        self.name, self.chunk_dir, l1name, l2name, err),
                };
                self.sweep_old_files(&mut l2_handle, status)?;
            }
        }
        Ok(())
    }

    pub fn insert_chunk(&self, chunk: &[u8]) -> Result<(bool, [u8; 32]), Error> {

        // fixme: use Sha512/256 when available
        let mut hasher = sha::Sha256::new();
        hasher.update(chunk);

        let digest = hasher.finish();

        //println!("DIGEST {}", digest_to_hex(&digest));

        let mut chunk_path = self.chunk_dir.clone();
        let prefix = digest_to_prefix(&digest);
        chunk_path.push(&prefix);
        let digest_str = digest_to_hex(&digest);
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

        println!("PATH {:?}", chunk_path);

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

    if let Err(_e) = std::fs::remove_dir_all(".testdir") { /* ignore */ }

    let chunk_store = ChunkStore::open("test", ".testdir");
    assert!(chunk_store.is_err());

    let chunk_store = ChunkStore::create("test", ".testdir").unwrap();
    let (exists, _) = chunk_store.insert_chunk(&[0u8, 1u8]).unwrap();
    assert!(!exists);

    let (exists, _) = chunk_store.insert_chunk(&[0u8, 1u8]).unwrap();
    assert!(exists);


    let chunk_store = ChunkStore::create("test", ".testdir");
    assert!(chunk_store.is_err());


}
