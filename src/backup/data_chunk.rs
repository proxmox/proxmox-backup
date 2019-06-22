use failure::*;
use std::convert::TryInto;
use proxmox::tools::io::ops::ReadExtOps;
use crate::tools::write::WriteUtilOps;

use super::*;

/// Data chunk with positional information
pub struct ChunkInfo {
    pub chunk: DataChunk,
    pub chunk_len: u64,
    pub offset: u64,
}

/// Data chunk binary storage format
///
/// Data chunks are identified by a unique digest, and can be
/// compressed and encrypted. A simply binary format is used to store
/// them on disk or transfer them over the network.
///
/// Please use the ``DataChunkBuilder`` to create new instances.
pub struct DataChunk {
    digest: [u8; 32],
    raw_data: Vec<u8>, // tagged, compressed, encryped data
}

impl DataChunk {

    /// accessor to raw_data field
    pub fn raw_data(&self) -> &[u8]  {
        &self.raw_data
    }

    /// accessor to chunk digest field
    pub fn digest(&self) -> &[u8; 32] {
        &self.digest
    }

    /// accessor to chunk type (magic number)
    pub fn magic(&self) -> &[u8; 8] {
        self.raw_data[0..8].try_into().unwrap()
    }

        /// accessor to crc32 checksum
    pub fn crc(&self) -> u32 {
        let crc_o = proxmox::tools::offsetof!(DataChunkHeader, crc);
        u32::from_le_bytes(self.raw_data[crc_o..crc_o+4].try_into().unwrap())
    }

    // set the CRC checksum field
    pub fn set_crc(&mut self, crc: u32) {
        let crc_o = proxmox::tools::offsetof!(DataChunkHeader, crc);
        self.raw_data[crc_o..crc_o+4].copy_from_slice(&crc.to_le_bytes());
    }

    /// compute the CRC32 checksum
    pub fn compute_crc(&mut self) -> u32 {
        let mut hasher = crc32fast::Hasher::new();
        let start = std::mem::size_of::<DataChunkHeader>(); // start after HEAD
        hasher.update(&self.raw_data[start..]);
        hasher.finalize()
    }

    fn encode(
        data: &[u8],
        config: Option<&CryptConfig>,
        digest: [u8;32],
        compress: bool,
    ) -> Result<Self, Error> {

        if let Some(config) = config {

            let compr_data;
            let (_compress, data, magic) = if compress {
                compr_data = zstd::block::compress(data, 1)?;
                // Note: We only use compression if result is shorter
                if compr_data.len() < data.len() {
                    (true, &compr_data[..], ENCR_COMPR_CHUNK_MAGIC_1_0)
                } else {
                    (false, data, ENCRYPTED_CHUNK_MAGIC_1_0)
                }
            } else {
                (false, data, ENCRYPTED_CHUNK_MAGIC_1_0)
            };

            let header_len = std::mem::size_of::<EncryptedDataChunkHeader>();
            let mut raw_data = Vec::with_capacity(data.len() + header_len);

            let dummy_head = EncryptedDataChunkHeader {
                head: DataChunkHeader { magic: [0u8; 8], crc: [0; 4] },
                iv: [0u8; 16],
                tag: [0u8; 16],
            };
            raw_data.write_value(&dummy_head)?;

            let (iv, tag) = config.encrypt_to(data, &mut raw_data)?;

            let head = EncryptedDataChunkHeader {
                head: DataChunkHeader { magic, crc: [0; 4] }, iv, tag,
            };

            (&mut raw_data[0..header_len]).write_value(&head)?;

            return Ok(DataChunk { digest, raw_data });
        } else {

            let max_data_len = data.len() + std::mem::size_of::<DataChunkHeader>();
            if compress {
                let mut comp_data = Vec::with_capacity(max_data_len);

                let head =  DataChunkHeader {
                    magic: COMPRESSED_CHUNK_MAGIC_1_0,
                    crc: [0; 4],
                };
                comp_data.write_value(&head)?;

                zstd::stream::copy_encode(data, &mut comp_data, 1)?;

                if comp_data.len() < max_data_len {
                    let chunk = DataChunk { digest, raw_data: comp_data };
                    return Ok(chunk);
                }
            }

            let mut raw_data = Vec::with_capacity(max_data_len);

            let head =  DataChunkHeader {
                magic: UNCOMPRESSED_CHUNK_MAGIC_1_0,
                crc: [0; 4],
            };
            raw_data.write_value(&head)?;
            raw_data.extend_from_slice(data);

            let chunk = DataChunk { digest, raw_data };
            return Ok(chunk);
        }
    }

    /// Decode chunk data
    pub fn decode(self, config: Option<&CryptConfig>) -> Result<Vec<u8>, Error> {

        let magic = self.magic();

        if magic == &UNCOMPRESSED_CHUNK_MAGIC_1_0 {
            let data_start = std::mem::size_of::<DataChunkHeader>();
            return Ok(self.raw_data[data_start..].to_vec());
        } else if magic == &COMPRESSED_CHUNK_MAGIC_1_0 {
            let data_start = std::mem::size_of::<DataChunkHeader>();
            let data = zstd::block::decompress(&self.raw_data[data_start..], 16*1024*1024)?;
            return Ok(data);
        } else if magic == &ENCR_COMPR_CHUNK_MAGIC_1_0 || magic == &ENCRYPTED_CHUNK_MAGIC_1_0 {
            let header_len = std::mem::size_of::<EncryptedDataChunkHeader>();
            let head = unsafe {
                (&self.raw_data[..header_len]).read_le_value::<EncryptedDataChunkHeader>()?
            };

            if let Some(config) = config  {
                let data = if magic == &ENCR_COMPR_CHUNK_MAGIC_1_0 {
                    config.decode_compressed_chunk(&self.raw_data[header_len..], &head.iv, &head.tag)?
                } else {
                    config.decode_uncompressed_chunk(&self.raw_data[header_len..], &head.iv, &head.tag)?
                };
                return Ok(data);
            } else {
                bail!("unable to decrypt chunk - missing CryptConfig");
            }
        } else {
            bail!("Invalid chunk magic number.");
        }
    }

    /// Load chunk data from ``reader``
    ///
    /// Please note that it is impossible to compute the digest for
    /// encrypted chunks, so we need to trust and use the provided
    /// ``digest``.
    pub fn load(reader: &mut dyn std::io::Read, digest: [u8; 32]) -> Result<Self, Error> {

        let mut data = Vec::with_capacity(1024*1024);
        reader.read_to_end(&mut data)?;

        Self::from_raw(data, digest)
    }

    /// Create Instance from raw data
    pub fn from_raw(data: Vec<u8>, digest: [u8;32]) -> Result<Self, Error> {

        if data.len() < std::mem::size_of::<DataChunkHeader>() {
            bail!("chunk too small ({} bytes).", data.len());
        }

        let magic = &data[0..8];

        if magic == ENCR_COMPR_CHUNK_MAGIC_1_0 || magic == ENCRYPTED_CHUNK_MAGIC_1_0 {

            if data.len() < std::mem::size_of::<EncryptedDataChunkHeader>() {
                bail!("encrypted chunk too small ({} bytes).", data.len());
            }

            let chunk = DataChunk { digest: digest, raw_data: data };

            Ok(chunk)
        } else if magic == COMPRESSED_CHUNK_MAGIC_1_0 || magic == UNCOMPRESSED_CHUNK_MAGIC_1_0 {

            let chunk = DataChunk { digest: digest, raw_data: data };

            Ok(chunk)
        } else {
            bail!("unable to parse raw chunk - wrong magic");
        }
    }

    /// Verify digest and data length for unencrypted chunks.
    ///
    /// To do that, we need to decompress data first. Please note that
    /// this is noth possible for encrypted chunks.
    pub fn verify_unencrypted(&self, expected_chunk_size: usize) -> Result<(), Error> {

        let magic = self.magic();

        let verify_raw_data = |data: &[u8]| {
            if expected_chunk_size != data.len() {
                bail!("detected chunk with wrong length ({} != {})", expected_chunk_size, data.len());
            }
            let digest = openssl::sha::sha256(data);
            if digest != self.digest {
                bail!("detected chunk with wrong digest.");
            }
            Ok(())
        };

        if magic == &COMPRESSED_CHUNK_MAGIC_1_0 {
           let data = zstd::block::decompress(&self.raw_data[12..], 16*1024*1024)?;
           verify_raw_data(&data)?;
        } else if magic == &UNCOMPRESSED_CHUNK_MAGIC_1_0 {
            verify_raw_data(&self.raw_data[12..])?;
        }

        Ok(())
    }
}

/// Builder for DataChunk
///
/// Main purpose is to centralize digest computation. Digest
/// computation differ for encryped chunk, and this interface ensures that
/// we always compute the correct one.
pub struct DataChunkBuilder<'a, 'b> {
    config: Option<&'b CryptConfig>,
    orig_data: &'a [u8],
    digest_computed: bool,
    digest: [u8; 32],
    compress: bool,
}

impl <'a, 'b> DataChunkBuilder<'a, 'b> {

    /// Create a new builder instance.
    pub fn new(orig_data: &'a [u8]) -> Self {
        Self {
            orig_data,
            config: None,
            digest_computed: false,
            digest: [0u8; 32],
            compress: true,
        }
    }

    /// Set compression flag.
    ///
    /// If true, chunk data is compressed using zstd (level 1).
    pub fn compress(mut self, value: bool) -> Self {
        self.compress = value;
        self
    }

    /// Set encryption Configuration
    ///
    /// If set, chunks are encrypted.
    pub fn crypt_config(mut self, value: &'b CryptConfig) -> Self {
        if self.digest_computed {
            panic!("unable to set crypt_config after compute_digest().");
        }
        self.config = Some(value);
        self
    }

    fn compute_digest(&mut self) {
        if !self.digest_computed {
            if let Some(config) = self.config {
                self.digest = config.compute_digest(self.orig_data);
            } else {
                self.digest = openssl::sha::sha256(self.orig_data);
            }
            self.digest_computed = true;
        }
    }

    /// Returns the chunk Digest
    ///
    /// Note: For encrypted chunks, this needs to be called after
    /// ``crypt_config``.
    pub fn digest(&mut self) -> &[u8; 32] {
        if !self.digest_computed {
            self.compute_digest();
        }
        &self.digest
    }

    /// Consume self and build the ``DataChunk``.
    pub fn build(mut self) -> Result<DataChunk, Error> {
        if !self.digest_computed {
            self.compute_digest();
        }

        let chunk = DataChunk::encode(
            self.orig_data,
            self.config,
            self.digest,
            self.compress,
        )?;

        Ok(chunk)
    }
}
