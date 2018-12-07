use failure::*;
use std::path::{Path, PathBuf};

pub struct ChunkStore {
    base: PathBuf,
    chunk_dir: PathBuf,
}

// see RFC4648 for base32hex definition
const BASE32HEX_ALPHABET: &'static [u8; 32]   = b"0123456789ABCDEFGHIJKLMNOPQRSTUV";

fn u10_to_base32hex(num: usize) -> String {
    let lo = num & 0b11111 as usize;
    let hi = num >> 5 as usize;
    format!("{}{}", BASE32HEX_ALPHABET[hi] as char, BASE32HEX_ALPHABET[lo] as char)
}

impl ChunkStore {

    fn new<P: Into<PathBuf>>(path: P) -> Self {
        let base = path.into();
        let mut chunk_dir = base.clone();
        chunk_dir.push(".chunks");

        ChunkStore { base, chunk_dir }
    }

    pub fn create<P: Into<PathBuf>>(path: P) -> Result<Self, Error> {

        let me = Self::new(path);

        std::fs::create_dir(&me.base)?;
        std::fs::create_dir(&me.chunk_dir)?;

        // create 1024 subdir
        for i in 0..1024 {
            let mut l1path = me.base.clone();
            l1path.push(u10_to_base32hex(i));
            std::fs::create_dir(&l1path)?;
        }

        Ok(me)
    }

    pub fn open<P: Into<PathBuf>>(path: P) -> Result<Self, Error> {

        let me = Self::new(path);

        let metadata = std::fs::metadata(&me.chunk_dir)?;

        println!("{:?}", metadata.file_type());

        Ok(me)
    }

}


#[test]
fn test_chunk_store1() {

    if let Err(_e) = std::fs::remove_dir_all(".testdir") { /* ignore */ }

    let chunk_store = ChunkStore::open(".testdir");
    assert!(chunk_store.is_err());

    let chunk_store = ChunkStore::create(".testdir");
    assert!(chunk_store.is_ok());

    let chunk_store = ChunkStore::create(".testdir");
    assert!(chunk_store.is_err());


}
