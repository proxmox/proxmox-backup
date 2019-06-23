use failure::*;
use std::convert::TryInto;

use proxmox::tools::io::ops::ReadExtOps;
use crate::tools::write::WriteUtilOps;

use super::*;

/// Data blob binary storage format
///
/// Data blobs store arbitrary binary data (< 16MB), and can be
/// compressed and encrypted. A simply binary format is used to store
/// them on disk or transfer them over the network. Please use index
/// files to store large data files (".fidx" of ".didx").
///
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

    /// accessor to crc32 checksum
    pub fn crc(&self) -> u32 {
        let crc_o = proxmox::tools::offsetof!(DataBlobHeader, crc);
        u32::from_le_bytes(self.raw_data[crc_o..crc_o+4].try_into().unwrap())
    }

    // set the CRC checksum field
    pub fn set_crc(&mut self, crc: u32) {
        let crc_o = proxmox::tools::offsetof!(DataBlobHeader, crc);
        self.raw_data[crc_o..crc_o+4].copy_from_slice(&crc.to_le_bytes());
    }

    /// compute the CRC32 checksum
    pub fn compute_crc(&mut self) -> u32 {
        let mut hasher = crc32fast::Hasher::new();
        let start = std::mem::size_of::<DataBlobHeader>(); // start after HEAD
        hasher.update(&self.raw_data[start..]);
        hasher.finalize()
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

            let compr_data;
            let (_compress, data, magic) = if compress {
                compr_data = zstd::block::compress(data, 1)?;
                // Note: We only use compression if result is shorter
                if compr_data.len() < data.len() {
                    (true, &compr_data[..], ENCR_COMPR_BLOB_MAGIC_1_0)
                } else {
                    (false, data, ENCRYPTED_BLOB_MAGIC_1_0)
                }
            } else {
                (false, data, ENCRYPTED_BLOB_MAGIC_1_0)
            };

            let header_len = std::mem::size_of::<EncryptedDataBlobHeader>();
            let mut raw_data = Vec::with_capacity(data.len() + header_len);

            let dummy_head = EncryptedDataBlobHeader {
                head: DataBlobHeader { magic: [0u8; 8], crc: [0; 4] },
                iv: [0u8; 16],
                tag: [0u8; 16],
            };
            raw_data.write_value(&dummy_head)?;

            let (iv, tag) = config.encrypt_to(data, &mut raw_data)?;

            let head = EncryptedDataBlobHeader {
                head: DataBlobHeader { magic, crc: [0; 4] }, iv, tag,
            };

            (&mut raw_data[0..header_len]).write_value(&head)?;

            return Ok(DataBlob { raw_data });
        } else {

            let max_data_len = data.len() + std::mem::size_of::<DataBlobHeader>();
            if compress {
                let mut comp_data = Vec::with_capacity(max_data_len);

                let head =  DataBlobHeader {
                    magic: COMPRESSED_BLOB_MAGIC_1_0,
                    crc: [0; 4],
                };
                comp_data.write_value(&head)?;

                zstd::stream::copy_encode(data, &mut comp_data, 1)?;

                if comp_data.len() < max_data_len {
                    return Ok(DataBlob { raw_data: comp_data });
                }
            }

            let mut raw_data = Vec::with_capacity(max_data_len);

            let head =  DataBlobHeader {
                magic: UNCOMPRESSED_BLOB_MAGIC_1_0,
                crc: [0; 4],
            };
            raw_data.write_value(&head)?;
            raw_data.extend_from_slice(data);

            return Ok(DataBlob { raw_data });
        }
    }

    /// Decode blob data
    pub fn decode(self, config: Option<&CryptConfig>) -> Result<Vec<u8>, Error> {

        let magic = self.magic();

        if magic == &UNCOMPRESSED_BLOB_MAGIC_1_0 {
            let data_start = std::mem::size_of::<DataBlobHeader>();
            return Ok(self.raw_data[data_start..].to_vec());
        } else if magic == &COMPRESSED_BLOB_MAGIC_1_0 {
            let data_start = std::mem::size_of::<DataBlobHeader>();
            let data = zstd::block::decompress(&self.raw_data[data_start..], 16*1024*1024)?;
            return Ok(data);
        } else if magic == &ENCR_COMPR_BLOB_MAGIC_1_0 || magic == &ENCRYPTED_BLOB_MAGIC_1_0 {
            let header_len = std::mem::size_of::<EncryptedDataBlobHeader>();
            let head = unsafe {
                (&self.raw_data[..header_len]).read_le_value::<EncryptedDataBlobHeader>()?
            };

            if let Some(config) = config  {
                let data = if magic == &ENCR_COMPR_BLOB_MAGIC_1_0 {
                    config.decode_compressed_chunk(&self.raw_data[header_len..], &head.iv, &head.tag)?
                } else {
                    config.decode_uncompressed_chunk(&self.raw_data[header_len..], &head.iv, &head.tag)?
                };
                return Ok(data);
            } else {
                bail!("unable to decrypt blob - missing CryptConfig");
            }
        } else {
            bail!("Invalid blob magic number.");
        }
    }

    /// Create Instance from raw data
    pub fn from_raw(data: Vec<u8>) -> Result<Self, Error> {

        if data.len() < std::mem::size_of::<DataBlobHeader>() {
            bail!("blob too small ({} bytes).", data.len());
        }

        let magic = &data[0..8];

        if magic == ENCR_COMPR_BLOB_MAGIC_1_0 || magic == ENCRYPTED_BLOB_MAGIC_1_0 {

            if data.len() < std::mem::size_of::<EncryptedDataBlobHeader>() {
                bail!("encrypted blob too small ({} bytes).", data.len());
            }

            let blob = DataBlob { raw_data: data };

            Ok(blob)
        } else if magic == COMPRESSED_BLOB_MAGIC_1_0 || magic == UNCOMPRESSED_BLOB_MAGIC_1_0 {

            let blob = DataBlob { raw_data: data };

            Ok(blob)
        } else {
            bail!("unable to parse raw blob - wrong magic");
        }
    }

}
