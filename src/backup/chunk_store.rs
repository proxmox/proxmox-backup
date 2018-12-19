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
    base: PathBuf,
    chunk_dir: PathBuf,
    hasher: Sha512Trunc256,
    mutex: Mutex<bool>,
    lockfile: File,
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

    pub fn create<P: Into<PathBuf>>(path: P) -> Result<Self, Error> {

        let base: PathBuf = path.into();
        let chunk_dir = Self::chunk_dir(&base);

        if let Err(err) = std::fs::create_dir(&base) {
            bail!("unable to create chunk store {:?} - {}", base, err);
        }

        if let Err(err) = std::fs::create_dir(&chunk_dir) {
            bail!("unable to create chunk store subdir {:?} - {}", chunk_dir, err);
        }

        // create 4096 subdir
        for i in 0..4096 {
            let mut l1path = chunk_dir.clone();
            l1path.push(format!("{:03x}",i));
            if let Err(err) = std::fs::create_dir(&l1path) {
                bail!("unable to create chunk subdir {:?} - {}", l1path, err);
            }
        }

        Self::open(base)
    }

    pub fn open<P: Into<PathBuf>>(path: P) -> Result<Self, Error> {

        let base: PathBuf = path.into();
        let chunk_dir = Self::chunk_dir(&base);

        if let Err(err) = std::fs::metadata(&chunk_dir) {
            bail!("unable to open chunk store {:?} - {}", chunk_dir, err);
        }

        let mut lockfile_path = base.clone();
        lockfile_path.push(".lock");

        // make sure only one process/thread/task can use it
        let lockfile = tools::open_file_locked(
            lockfile_path, Duration::from_secs(10))?;

        Ok(ChunkStore {
            base,
            chunk_dir,
            hasher: Sha512Trunc256::new(),
            lockfile,
            mutex: Mutex::new(false)
        })
    }

    pub fn touch_chunk(&mut self, digest:&[u8]) ->  Result<(), Error> {

        // fixme:  nix::sys::stat::utimensat
        let mut chunk_path = self.chunk_dir.clone();
        let prefix = digest_to_prefix(&digest);
        chunk_path.push(&prefix);
        let digest_str = digest_to_hex(&digest);
        chunk_path.push(&digest_str);

        std::fs::metadata(&chunk_path)?;
        Ok(())
    }

    fn sweep_old_files(&self, dir: &Path) {

        let mut handle = match nix::dir::Dir::open(
            dir, nix::fcntl::OFlag::O_RDONLY, nix::sys::stat::Mode::empty()) {
            Ok(h) => h,
            Err(_) => return,
        };

        let rawfd = handle.as_raw_fd();

        let now = unsafe { libc::time(std::ptr::null_mut()) };

        for entry in handle.iter() {
             match entry {
                Ok(entry) => {
                    if let Some(file_type) = entry.file_type() {
                        if file_type == nix::dir::Type::File {
                            let filename = entry.file_name();
                            if let Ok(stat) = nix::sys::stat::fstatat(rawfd, filename, nix::fcntl::AtFlags::AT_SYMLINK_NOFOLLOW) {
                                let age = now - stat.st_atime;
                                println!("FOUND {}  {:?}", age/(3600*24), filename);
                                if age/(3600*24) >= 2 {
                                    println!("UNLINK {}  {:?}", age/(3600*24), filename);
                                    unsafe { libc::unlinkat(rawfd, filename.as_ptr(), 0); }
                                }
                            }
                        }
                    }
                }
                Err(_) => {
                    // fixme ??
                }
             }
         }

    }

    pub fn sweep_used_chunks(&mut self) -> Result<(), Error> {

        for i in 0..4096 {
            let mut l1path = self.chunk_dir.clone();
            l1path.push(format!("{:03x}", i));
            for j in 0..256 {
                let mut l2path = l1path.clone();
                l2path.push(format!("{:02x}", j));
                self.sweep_old_files(&l2path);
            }
        }

        Ok(())
    }

    pub fn insert_chunk(&mut self, chunk: &[u8]) -> Result<(bool, [u8; 32]), Error> {

        self.hasher.reset();
        self.hasher.input(chunk);

        let mut digest = [0u8; 32];
        self.hasher.result(&mut digest);
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
                bail!("Got unexpected file type for chunk {}", digest_str);
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
            bail!("Atomic rename failed for chunk {} - {}", digest_str, err);
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

    let chunk_store = ChunkStore::open(".testdir");
    assert!(chunk_store.is_err());

    let mut chunk_store = ChunkStore::create(".testdir").unwrap();
    let (exists, _) = chunk_store.insert_chunk(&[0u8, 1u8]).unwrap();
    assert!(!exists);

    let (exists, _) = chunk_store.insert_chunk(&[0u8, 1u8]).unwrap();
    assert!(exists);


    let chunk_store = ChunkStore::create(".testdir");
    assert!(chunk_store.is_err());


}
