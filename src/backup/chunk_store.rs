use failure::*;
use std::path::{Path, PathBuf};
use std::io::Write;
use std::time::Duration;

use crypto::digest::Digest;
use crypto::sha2::Sha512Trunc256;
use std::sync::Mutex;

use std::fs::File;
use std::os::unix::io::AsRawFd;

use crate::tools;

pub struct ChunkStore {
    name: String, // used for error reporting
    base: PathBuf,
    chunk_dir: PathBuf,
    mutex: Mutex<bool>,
    _lockfile: File,
}

const HEX_CHARS: &'static [u8; 16] = b"0123456789abcdef";

pub fn digest_to_hex(digest: &[u8]) -> String {

    let mut buf = Vec::<u8>::with_capacity(digest.len()*2);

    for i in 0..digest.len() {
        buf.push(HEX_CHARS[(digest[i] >> 4) as usize]);
        buf.push(HEX_CHARS[(digest[i] & 0xf) as usize]);
    }

    unsafe { String::from_utf8_unchecked(buf) }
}

fn digest_to_prefix(digest: &[u8]) -> PathBuf {

    let mut buf = Vec::<u8>::with_capacity(3+1+2+1);

    buf.push(HEX_CHARS[(digest[0] as usize) >> 4]);
    buf.push(HEX_CHARS[(digest[0] as usize) &0xf]);
    buf.push(HEX_CHARS[(digest[1] as usize) >> 4]);
    buf.push('/' as u8);

    buf.push(HEX_CHARS[(digest[1] as usize) & 0xf]);
    buf.push(HEX_CHARS[(digest[2] as usize) >> 4]);
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

        // create 4096 subdir
        for i in 0..4096 {
            let mut l1path = chunk_dir.clone();
            l1path.push(format!("{:03x}",i));
            if let Err(err) = std::fs::create_dir(&l1path) {
                bail!("unable to create chunk store '{}' subdir {:?} - {}", name, l1path, err);
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

        // fixme:  nix::sys::stat::utimensat
        let mut chunk_path = self.chunk_dir.clone();
        let prefix = digest_to_prefix(&digest);
        chunk_path.push(&prefix);
        let digest_str = digest_to_hex(&digest);
        chunk_path.push(&digest_str);

        std::fs::metadata(&chunk_path)?;
        Ok(())
    }

    fn sweep_old_files(&self, handle: &mut nix::dir::Dir) -> Result<(), Error> {

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
                println!("FOUND {}  {:?}", age/(3600*24), filename);
                if age/(3600*24) >= 2 {
                    println!("UNLINK {}  {:?}", age/(3600*24), filename);
                    let res = unsafe { libc::unlinkat(rawfd, filename.as_ptr(), 0) };
                    if res != 0 {
                        let err = nix::Error::last();
                        bail!("unlink chunk {:?} failed on store '{}' - {}", filename, self.name, err);
                    }
                }
            }
        }
        Ok(())
    }

    pub fn sweep_used_chunks(&self) -> Result<(), Error> {

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

        for i in 0..4096 {
            let l1name = PathBuf::from(format!("{:03x}", i));
            let mut l1_handle = match nix::dir::Dir::openat(
                base_fd, &l1name, OFlag::O_RDONLY, Mode::empty()) {
                Ok(h) => h,
                Err(err) => bail!("unable to open store '{}' dir {:?}/{:?} - {}",
                                  self.name, self.chunk_dir, l1name, err),
            };

            let l1_fd = l1_handle.as_raw_fd();

            for l1_entry in l1_handle.iter() {
                let l1_entry = match l1_entry {
                    Ok(l1_entry) => l1_entry,
                    Err(_) => continue /* ignore errors? */,
                };
                let file_type = match l1_entry.file_type() {
                    Some(file_type) => file_type,
                    None => bail!("unsupported file system type on chunk store '{}'", self.name),
                };
                if file_type != nix::dir::Type::Directory { continue; }

                let l2name = l1_entry.file_name();
                if l2name.to_bytes_with_nul()[0] == b'.' { continue; }

                let mut l2_handle = match Dir::openat(
                    l1_fd, l2name, OFlag::O_RDONLY, Mode::empty()) {
                    Ok(h) => h,
                    Err(err) => bail!(
                        "unable to open store '{}' dir {:?}/{:?}/{:?} - {}",
                        self.name, self.chunk_dir, l1name, l2name, err),
                };
                self.sweep_old_files(&mut l2_handle)?;
            }
        }
        Ok(())
    }

    pub fn insert_chunk(&self, chunk: &[u8]) -> Result<(bool, [u8; 32]), Error> {
        let mut hasher = Sha512Trunc256::new();
        hasher.input(chunk);

        let mut digest = [0u8; 32];
        hasher.result(&mut digest);
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

        let mut chunk_dir = self.chunk_dir.clone();
        chunk_dir.push(&prefix);

        if let Err(_) = std::fs::create_dir(&chunk_dir) { /* ignore */ }

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
