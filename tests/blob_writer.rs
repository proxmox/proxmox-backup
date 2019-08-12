use failure::*;
use std::io::Cursor;
use std::io::Write;

use proxmox_backup::backup::*;

#[test]
fn test_data_blob_writer() -> Result<(), Error> {

    let key = [1u8; 32];
    let crypt_config = CryptConfig::new(key)?;

    let test_data = b"123456789".to_vec();

    let verify_test_blob = |raw_data: Vec<u8>| -> Result<(), Error> {
        let blob = DataBlob::from_raw(raw_data)?;
        blob.verify_crc()?;
        
        let data = blob.decode(Some(&crypt_config))?;
        if data != test_data {
            bail!("blob data is wrong");
        }
        Ok(())
    };

    {
        let tmp = Cursor::new(Vec::<u8>::new()); 
        let mut blob_writer = DataBlobWriter::new_uncompressed(tmp)?;
        blob_writer.write_all(&test_data)?;

        let raw_data = blob_writer.finish()?.into_inner();

        println!("UNCOMPRESSED: {:?}", raw_data);
        verify_test_blob(raw_data)?;
    }

    {
        let tmp = Cursor::new(Vec::<u8>::new()); 
        let mut blob_writer = DataBlobWriter::new_compressed(tmp)?;
        blob_writer.write_all(&test_data)?;

        let raw_data = blob_writer.finish()?.into_inner();

        println!("COMPRESSED: {:?}", raw_data);
        verify_test_blob(raw_data)?;
    }

    {
        let tmp = Cursor::new(Vec::<u8>::new()); 
        let mut blob_writer = DataBlobWriter::new_signed(tmp, &crypt_config)?;
        blob_writer.write_all(&test_data)?;

        let raw_data = blob_writer.finish()?.into_inner();

        println!("SIGNED: {:?}", raw_data);
        verify_test_blob(raw_data)?;
    }

    {
        let tmp = Cursor::new(Vec::<u8>::new()); 
        let mut blob_writer = DataBlobWriter::new_signed_compressed(tmp, &crypt_config)?;
        blob_writer.write_all(&test_data)?;

        let raw_data = blob_writer.finish()?.into_inner();

        println!("SIGNED COMPR: {:?}", raw_data);
        verify_test_blob(raw_data)?;
    }

    {
        let tmp = Cursor::new(Vec::<u8>::new()); 
        let mut blob_writer = DataBlobWriter::new_encrypted(tmp, &crypt_config)?;
        blob_writer.write_all(&test_data)?;

        let raw_data = blob_writer.finish()?.into_inner();

        println!("ENCRYPTED: {:?}", raw_data);
        verify_test_blob(raw_data)?;
    }

    {
        let tmp = Cursor::new(Vec::<u8>::new()); 
        let mut blob_writer = DataBlobWriter::new_encrypted_compressed(tmp, &crypt_config)?;
        blob_writer.write_all(&test_data)?;

        let raw_data = blob_writer.finish()?.into_inner();

        println!("ENCRYPTED COMPR: {:?}", raw_data);
        verify_test_blob(raw_data)?;
    }

    Ok(())
}
