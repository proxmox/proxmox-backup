use failure::*;
use std::sync::Arc;
use std::io::Cursor;
use std::io::{Read, Write, Seek, SeekFrom };
use lazy_static::lazy_static;

use proxmox_backup::backup::*;

lazy_static! {
    static ref TEST_DATA: Vec<u8> = {
        let mut data = Vec::new();

        for i in 0..100_000 {
            data.push((i%255) as u8);
        }

        data
    };

    static ref CRYPT_CONFIG: Arc<CryptConfig> = {
        let key = [1u8; 32];
        Arc::new(CryptConfig::new(key).unwrap())
    };
}

fn verify_test_blob(mut cursor: Cursor<Vec<u8>>) -> Result<(), Error> {

    // run read tests with different buffer sizes
    for size in [1, 3, 64*1024].iter() {

        println!("Starting DataBlobReader test (size = {})", size);

        cursor.seek(SeekFrom::Start(0))?;
        let mut reader = DataBlobReader::new(&mut cursor, Some(CRYPT_CONFIG.clone()))?;
        let mut buffer = Vec::<u8>::new();
        // read the whole file
        //reader.read_to_end(&mut buffer)?;
        let mut buf = vec![0u8; *size];
        loop {
            let count = reader.read(&mut buf)?;
            if count == 0 { break; }
            buffer.extend(&buf[..count]);
        }

        reader.finish()?;
        if buffer != *TEST_DATA {
            bail!("blob data is wrong (read buffer size {})", size);
        }
    }

    let raw_data = cursor.into_inner();

    let blob = DataBlob::from_raw(raw_data)?;
    blob.verify_crc()?;

    let data = blob.decode(Some(CRYPT_CONFIG.clone()))?;
    if data != *TEST_DATA {
        bail!("blob data is wrong (decode)");
    }
    Ok(())
}

#[test]
fn test_uncompressed_blob_writer() -> Result<(), Error> {
    let tmp = Cursor::new(Vec::<u8>::new());
    let mut blob_writer = DataBlobWriter::new_uncompressed(tmp)?;
    blob_writer.write_all(&TEST_DATA)?;

    verify_test_blob(blob_writer.finish()?)
}

#[test]
fn test_compressed_blob_writer() -> Result<(), Error> {
    let tmp = Cursor::new(Vec::<u8>::new());
    let mut blob_writer = DataBlobWriter::new_compressed(tmp)?;
    blob_writer.write_all(&TEST_DATA)?;

    verify_test_blob(blob_writer.finish()?)
}

#[test]
fn test_signed_blob_writer() -> Result<(), Error> {
    let tmp = Cursor::new(Vec::<u8>::new());
    let mut blob_writer = DataBlobWriter::new_signed(tmp, CRYPT_CONFIG.clone())?;
    blob_writer.write_all(&TEST_DATA)?;

    verify_test_blob(blob_writer.finish()?)
}

#[test]
fn test_signed_compressed_blob_writer() -> Result<(), Error> {
    let tmp = Cursor::new(Vec::<u8>::new());
    let mut blob_writer = DataBlobWriter::new_signed_compressed(tmp, CRYPT_CONFIG.clone())?;
    blob_writer.write_all(&TEST_DATA)?;

    verify_test_blob(blob_writer.finish()?)
}

#[test]
fn test_encrypted_blob_writer() -> Result<(), Error> {
    let tmp = Cursor::new(Vec::<u8>::new());
    let mut blob_writer = DataBlobWriter::new_encrypted(tmp, CRYPT_CONFIG.clone())?;
    blob_writer.write_all(&TEST_DATA)?;

    verify_test_blob(blob_writer.finish()?)
}

#[test]
fn test_encrypted_compressed_blob_writer() -> Result<(), Error> {
    let tmp = Cursor::new(Vec::<u8>::new());
    let mut blob_writer = DataBlobWriter::new_encrypted_compressed(tmp, CRYPT_CONFIG.clone())?;
    blob_writer.write_all(&TEST_DATA)?;

    verify_test_blob(blob_writer.finish()?)
}
