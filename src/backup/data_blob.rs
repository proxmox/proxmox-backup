use failure::*;
use std::convert::TryInto;
use std::io::Write;

use super::*;

/// Data blob binary storage format
///
/// Data blobs store arbitrary binary data (< 16MB), and can be
/// compressed and encrypted. A simply binary format is used to store
/// them on disk or transfer them over the network. Please use index
/// files to store large data files (".fidx" of ".didx").
///
/// The format start with a 8 byte magic number to identify the type.
/// Encrypted blobs contain a 16 byte IV, followed by a 18 byte AD
/// tag, followed by the encrypted data (MAGIC || IV || TAG ||
/// EncryptedData).
///
/// Unencrypted blobs simply contain the (compressed) data.
///
/// This is basically the same format we use for ``DataChunk``, but
/// with other magic numbers so that we can distinguish them.
pub struct DataBlob {
    raw_data: Vec<u8>, // tagged, compressed, encryped data
}

impl DataBlob {

    /// accessor to raw_data field
    pub fn raw_data(&self) -> &[u8]  {
        &self.raw_data
    }

    /// accessor to chunk type (magic number)
    pub fn magic(&self) -> &[u8; 8] {
        self.raw_data[0..8].try_into().unwrap()
    }

    pub fn encode(
        data: &[u8],
        config: Option<&CryptConfig>,
        compress: bool,
    ) -> Result<Self, Error> {

        if data.len() > 16*1024*1024 {
            bail!("data blob too large ({} bytes).", data.len());
        }

        if let Some(config) = config {

            let enc_data = config.encode_chunk(
                data,
                compress,
                &ENCRYPTED_BLOB_MAGIC_1_0,
                &ENCR_COMPR_BLOB_MAGIC_1_0,
            )?;
            return Ok(DataBlob { raw_data: enc_data });
        } else {

            if compress {
                let mut comp_data = Vec::with_capacity(data.len() + 8);

                comp_data.write_all(&COMPRESSED_BLOB_MAGIC_1_0)?;
                zstd::stream::copy_encode(data, &mut comp_data, 1)?;

                if comp_data.len() < (data.len() + 8) {
                    return Ok(DataBlob { raw_data: comp_data });
                }
            }

            let mut raw_data = Vec::with_capacity(data.len() + 8);

            raw_data.write_all(&UNCOMPRESSED_BLOB_MAGIC_1_0)?;
            raw_data.extend_from_slice(data);

            return Ok(DataBlob { raw_data });
        }
    }

    /// Decode blob data
    pub fn decode(self, config: Option<&CryptConfig>) -> Result<Vec<u8>, Error> {

        let magic = self.magic();

        if magic == &UNCOMPRESSED_BLOB_MAGIC_1_0 {
             return Ok(self.raw_data);
        } else if magic == &COMPRESSED_BLOB_MAGIC_1_0 {

            let data = zstd::block::decompress(&self.raw_data[8..], 16*1024*1024)?;
            return Ok(data);

        } else if magic == &ENCR_COMPR_BLOB_MAGIC_1_0 || magic == &ENCRYPTED_BLOB_MAGIC_1_0 {
            if let Some(config) = config  {
                let data = if magic == &ENCR_COMPR_BLOB_MAGIC_1_0 {
                    config.decode_compressed_chunk(&self.raw_data)?
                } else {
                    config.decode_uncompressed_chunk(&self.raw_data)?
                };
                return Ok(data);
            } else {
                bail!("unable to decrypt blob - missing CryptConfig");
            }
        } else {
            bail!("Invalid blob magic number.");
        }
    }
}
