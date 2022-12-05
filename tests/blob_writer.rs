use std::io::Cursor;
use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::Arc;

use anyhow::{bail, Error};
use lazy_static::lazy_static;

use pbs_datastore::{DataBlob, DataBlobReader, DataBlobWriter};
use pbs_tools::crypt_config::CryptConfig;

lazy_static! {
    static ref TEST_DATA: Vec<u8> = {
        let mut data = Vec::new();

        for i in 0..100_000 {
            data.push((i % 255) as u8);
        }

        data
    };
    static ref CRYPT_CONFIG: Arc<CryptConfig> = {
        let key = [1u8; 32];
        Arc::new(CryptConfig::new(key).unwrap())
    };
    static ref TEST_DIGEST_PLAIN: [u8; 32] = [
        83, 154, 96, 195, 167, 204, 38, 142, 204, 224, 130, 201, 24, 71, 2, 188, 130, 155, 177, 6,
        162, 100, 61, 238, 38, 219, 63, 240, 191, 132, 87, 238
    ];
    static ref TEST_DIGEST_ENC: [u8; 32] = [
        50, 162, 191, 93, 255, 132, 9, 14, 127, 23, 92, 39, 246, 102, 245, 204, 130, 104, 4, 106,
        182, 239, 218, 14, 80, 17, 150, 188, 239, 253, 198, 117
    ];
}

fn verify_test_blob(mut cursor: Cursor<Vec<u8>>, digest: &[u8; 32]) -> Result<(), Error> {
    // run read tests with different buffer sizes
    for size in [1, 3, 64 * 1024].iter() {
        println!("Starting DataBlobReader test (size = {})", size);

        cursor.seek(SeekFrom::Start(0))?;
        let mut reader = DataBlobReader::new(&mut cursor, Some(CRYPT_CONFIG.clone()))?;
        let mut buffer = Vec::<u8>::new();
        // read the whole file
        //reader.read_to_end(&mut buffer)?;
        let mut buf = vec![0u8; *size];
        loop {
            let count = reader.read(&mut buf)?;
            if count == 0 {
                break;
            }
            buffer.extend(&buf[..count]);
        }

        reader.finish()?;
        if buffer != *TEST_DATA {
            bail!("blob data is wrong (read buffer size {})", size);
        }
    }

    let raw_data = cursor.into_inner();

    let blob = DataBlob::load_from_reader(&mut &raw_data[..])?;

    let data = blob.decode(Some(&CRYPT_CONFIG), Some(digest))?;
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

    verify_test_blob(blob_writer.finish()?, &TEST_DIGEST_PLAIN)
}

#[test]
fn test_compressed_blob_writer() -> Result<(), Error> {
    let tmp = Cursor::new(Vec::<u8>::new());
    let mut blob_writer = DataBlobWriter::new_compressed(tmp)?;
    blob_writer.write_all(&TEST_DATA)?;

    verify_test_blob(blob_writer.finish()?, &TEST_DIGEST_PLAIN)
}

#[test]
fn test_encrypted_blob_writer() -> Result<(), Error> {
    let tmp = Cursor::new(Vec::<u8>::new());
    let mut blob_writer = DataBlobWriter::new_encrypted(tmp, CRYPT_CONFIG.clone())?;
    blob_writer.write_all(&TEST_DATA)?;

    verify_test_blob(blob_writer.finish()?, &TEST_DIGEST_ENC)
}

#[test]
fn test_encrypted_compressed_blob_writer() -> Result<(), Error> {
    let tmp = Cursor::new(Vec::<u8>::new());
    let mut blob_writer = DataBlobWriter::new_encrypted_compressed(tmp, CRYPT_CONFIG.clone())?;
    blob_writer.write_all(&TEST_DATA)?;

    verify_test_blob(blob_writer.finish()?, &TEST_DIGEST_ENC)
}
