use failure::*;
use std::path::{Path, PathBuf};
use std::io::Write;

use crypto::digest::Digest;
use crypto::sha2::Sha512Trunc256;
use std::sync::Mutex;

use std::fs::{File, OpenOptions};
use nix::fcntl::{flock, FlockArg};
use std::os::unix::io::AsRawFd;

pub struct ChunkStore {
    base: PathBuf,
    chunk_dir: PathBuf,
    hasher: Sha512Trunc256,
    mutex: Mutex<bool>,
    lockfile: File,
}

const HEX_CHARS: &'static [u8; 16] = b"0123456789abcdef";

fn u256_to_hex(digest: &[u8; 32]) -> String {

    let mut buf = Vec::<u8>::with_capacity(64);

    for i in 0..32 {
        buf.push(HEX_CHARS[(digest[i] >> 4) as usize]);
        buf.push(HEX_CHARS[(digest[i] & 0xf) as usize]);
    }

    unsafe { String::from_utf8_unchecked(buf) }
}

fn u256_to_prefix(digest: &[u8; 32]) -> PathBuf {

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

fn lock_file<P: AsRef<Path>>(filename: P, timeout: usize) -> Result<File, Error> {

    let path = filename.as_ref();
    let lockfile = match OpenOptions::new()
        .create(true)
        .append(true)
        .open(path) {
            Ok(file) => file,
            Err(err) => bail!("Unable to open lock {:?} - {}",
                              path, err),
        };

    let fd = lockfile.as_raw_fd();

    let now = std::time::SystemTime::now();
    let mut print_msg = true;
    loop {
        match flock(fd, FlockArg::LockExclusiveNonblock) {
            Ok(_) => break,
            Err(_) => {
                if print_msg {
                    print_msg = false;
                    eprintln!("trying to aquire lock...");
                }
            }
        }

        match now.elapsed() {
            Ok(elapsed) => {
                if elapsed.as_secs() >= (timeout as u64) {
                    bail!("unable to aquire lock {:?} - got timeout", path);
                }
            }
            Err(err) => {
                bail!("unable to aquire lock {:?} - clock problems - {}", path, err);
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    Ok(lockfile)
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
            let mut l1path = base.clone();
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
        let lockfile = lock_file(lockfile_path, 10)?;

        Ok(ChunkStore {
            base,
            chunk_dir,
            hasher: Sha512Trunc256::new(),
            lockfile,
            mutex: Mutex::new(false)
        })
    }

    pub fn insert_chunk(&mut self, chunk: &[u8]) -> Result<(bool, [u8; 32]), Error> {

        self.hasher.reset();
        self.hasher.input(chunk);

        let mut digest = [0u8; 32];
        self.hasher.result(&mut digest);
        println!("DIGEST {}", u256_to_hex(&digest));

        let mut chunk_path = self.base.clone();
        let prefix = u256_to_prefix(&digest);
        chunk_path.push(&prefix);
        let digest_str = u256_to_hex(&digest);
        chunk_path.push(&digest_str);

        let lock = self.mutex.lock();

        if let Ok(metadata) = std::fs::metadata(&chunk_path) {
            if metadata.is_file() {
                 return Ok((true, digest));
            } else {
                bail!("Got unexpected file type for chunk {}", digest_str);
            }
        }

        let mut chunk_dir = self.base.clone();
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
