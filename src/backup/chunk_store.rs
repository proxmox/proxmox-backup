use failure::*;
use std::path::{Path, PathBuf};
use std::io::Write;

use crypto::digest::Digest;
use crypto::sha2::Sha512Trunc256;

pub struct ChunkStore {
    base: PathBuf,
    chunk_dir: PathBuf,
    hasher: Sha512Trunc256,

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

impl ChunkStore {

    fn new<P: Into<PathBuf>>(path: P) -> Self {
        let base = path.into();
        let mut chunk_dir = base.clone();
        chunk_dir.push(".chunks");

        let hasher = Sha512Trunc256::new();

        ChunkStore { base, chunk_dir, hasher }
    }

    pub fn create<P: Into<PathBuf>>(path: P) -> Result<Self, Error> {

        let me = Self::new(path);

        std::fs::create_dir(&me.base)?;
        std::fs::create_dir(&me.chunk_dir)?;

        // create 4096 subdir
        for i in 0..4096 {
            let mut l1path = me.base.clone();
            l1path.push(format!("{:03x}",i));
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

    pub fn insert_chunk(&mut self, chunk: &[u8]) -> Result<([u8; 32]), Error> {

        self.hasher.reset();
        self.hasher.input(chunk);

        let mut digest = [0u8; 32];
        self.hasher.result(&mut digest);
        println!("DIGEST {}", u256_to_hex(&digest));

        let mut chunk_path = self.base.clone();

        let prefix = u256_to_prefix(&digest);
        chunk_path.push(prefix);

        if let Err(_e) = std::fs::create_dir(&chunk_path) { /* ignore */ }

        println!("PATH {:?}", chunk_path);

        chunk_path.push(u256_to_hex(&digest));
        //chunk_path.set_extension("tmp");
        //chunk_path.set_extension("");
        let mut f = std::fs::File::create(&chunk_path)?;
        f.write_all(chunk)?;

        println!("PATH {:?}", chunk_path);

        Ok(digest)
    }

}


#[test]
fn test_chunk_store1() {

    if let Err(_e) = std::fs::remove_dir_all(".testdir") { /* ignore */ }

    let chunk_store = ChunkStore::open(".testdir");
    assert!(chunk_store.is_err());

    let mut chunk_store = ChunkStore::create(".testdir").unwrap();
    chunk_store.insert_chunk(&[0u8, 1u8]);


    let chunk_store = ChunkStore::create(".testdir");
    assert!(chunk_store.is_err());


}
