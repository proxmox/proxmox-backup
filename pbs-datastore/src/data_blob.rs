use std::io::Write;

use anyhow::{bail, Error};
use openssl::symm::{decrypt_aead, Mode};

use proxmox_io::{ReadExt, WriteExt};

use pbs_api_types::CryptMode;
use pbs_tools::crypt_config::CryptConfig;

use super::file_formats::*;

const MAX_BLOB_SIZE: usize = 128 * 1024 * 1024;

/// Encoded data chunk with digest and positional information
pub struct ChunkInfo {
    pub chunk: DataBlob,
    pub digest: [u8; 32],
    pub chunk_len: u64,
    pub offset: u64,
}

/// Data blob binary storage format
///
/// Data blobs store arbitrary binary data (< 128MB), and can be
/// compressed and encrypted (or just signed). A simply binary format
/// is used to store them on disk or transfer them over the network.
///
/// Please use index files to store large data files (".fidx" of
/// ".didx").
///
pub struct DataBlob {
    raw_data: Vec<u8>, // tagged, compressed, encryped data
}

impl DataBlob {
    /// accessor to raw_data field
    pub fn raw_data(&self) -> &[u8] {
        &self.raw_data
    }

    /// Returns raw_data size
    pub fn raw_size(&self) -> u64 {
        self.raw_data.len() as u64
    }

    /// Consume self and returns raw_data
    pub fn into_inner(self) -> Vec<u8> {
        self.raw_data
    }

    /// accessor to chunk type (magic number)
    pub fn magic(&self) -> &[u8; 8] {
        self.raw_data[0..8].try_into().unwrap()
    }

    /// accessor to crc32 checksum
    pub fn crc(&self) -> u32 {
        let crc_o = proxmox_lang::offsetof!(DataBlobHeader, crc);
        u32::from_le_bytes(self.raw_data[crc_o..crc_o + 4].try_into().unwrap())
    }

    // set the CRC checksum field
    pub fn set_crc(&mut self, crc: u32) {
        let crc_o = proxmox_lang::offsetof!(DataBlobHeader, crc);
        self.raw_data[crc_o..crc_o + 4].copy_from_slice(&crc.to_le_bytes());
    }

    /// compute the CRC32 checksum
    pub fn compute_crc(&self) -> u32 {
        let mut hasher = crc32fast::Hasher::new();
        let start = header_size(self.magic()); // start after HEAD
        hasher.update(&self.raw_data[start..]);
        hasher.finalize()
    }

    // verify the CRC32 checksum
    pub fn verify_crc(&self) -> Result<(), Error> {
        let expected_crc = self.compute_crc();
        if expected_crc != self.crc() {
            bail!("Data blob has wrong CRC checksum.");
        }
        Ok(())
    }

    /// Create a DataBlob, optionally compressed and/or encrypted
    pub fn encode(
        data: &[u8],
        config: Option<&CryptConfig>,
        compress: bool,
    ) -> Result<Self, Error> {
        if data.len() > MAX_BLOB_SIZE {
            bail!("data blob too large ({} bytes).", data.len());
        }

        let mut blob = if let Some(config) = config {
            let compr_data;
            let (_compress, data, magic) = if compress {
                compr_data = zstd::bulk::compress(data, 1)?;
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
                head: DataBlobHeader {
                    magic: [0u8; 8],
                    crc: [0; 4],
                },
                iv: [0u8; 16],
                tag: [0u8; 16],
            };
            unsafe {
                raw_data.write_le_value(dummy_head)?;
            }

            let (iv, tag) = Self::encrypt_to(config, data, &mut raw_data)?;

            let head = EncryptedDataBlobHeader {
                head: DataBlobHeader { magic, crc: [0; 4] },
                iv,
                tag,
            };

            unsafe {
                (&mut raw_data[0..header_len]).write_le_value(head)?;
            }

            DataBlob { raw_data }
        } else {
            let max_data_len = data.len() + std::mem::size_of::<DataBlobHeader>();
            if compress {
                let mut comp_data = Vec::with_capacity(max_data_len);

                let head = DataBlobHeader {
                    magic: COMPRESSED_BLOB_MAGIC_1_0,
                    crc: [0; 4],
                };
                unsafe {
                    comp_data.write_le_value(head)?;
                }

                zstd::stream::copy_encode(data, &mut comp_data, 1)?;

                if comp_data.len() < max_data_len {
                    let mut blob = DataBlob {
                        raw_data: comp_data,
                    };
                    blob.set_crc(blob.compute_crc());
                    return Ok(blob);
                }
            }

            let mut raw_data = Vec::with_capacity(max_data_len);

            let head = DataBlobHeader {
                magic: UNCOMPRESSED_BLOB_MAGIC_1_0,
                crc: [0; 4],
            };
            unsafe {
                raw_data.write_le_value(head)?;
            }
            raw_data.extend_from_slice(data);

            DataBlob { raw_data }
        };

        blob.set_crc(blob.compute_crc());

        Ok(blob)
    }

    /// Get the encryption mode for this blob.
    pub fn crypt_mode(&self) -> Result<CryptMode, Error> {
        let magic = self.magic();

        Ok(
            if magic == &UNCOMPRESSED_BLOB_MAGIC_1_0 || magic == &COMPRESSED_BLOB_MAGIC_1_0 {
                CryptMode::None
            } else if magic == &ENCR_COMPR_BLOB_MAGIC_1_0 || magic == &ENCRYPTED_BLOB_MAGIC_1_0 {
                CryptMode::Encrypt
            } else {
                bail!("Invalid blob magic number.");
            },
        )
    }

    /// Decode blob data
    pub fn decode(
        &self,
        config: Option<&CryptConfig>,
        digest: Option<&[u8; 32]>,
    ) -> Result<Vec<u8>, Error> {
        let magic = self.magic();

        if magic == &UNCOMPRESSED_BLOB_MAGIC_1_0 {
            let data_start = std::mem::size_of::<DataBlobHeader>();
            let data = self.raw_data[data_start..].to_vec();
            if let Some(digest) = digest {
                Self::verify_digest(&data, None, digest)?;
            }
            Ok(data)
        } else if magic == &COMPRESSED_BLOB_MAGIC_1_0 {
            let data_start = std::mem::size_of::<DataBlobHeader>();
            let mut reader = &self.raw_data[data_start..];
            let data = zstd::stream::decode_all(&mut reader)?;
            // zstd::block::decompress is abou 10% slower
            // let data = zstd::block::decompress(&self.raw_data[data_start..], MAX_BLOB_SIZE)?;
            if let Some(digest) = digest {
                Self::verify_digest(&data, None, digest)?;
            }
            Ok(data)
        } else if magic == &ENCR_COMPR_BLOB_MAGIC_1_0 || magic == &ENCRYPTED_BLOB_MAGIC_1_0 {
            let header_len = std::mem::size_of::<EncryptedDataBlobHeader>();
            let head = unsafe {
                (&self.raw_data[..header_len]).read_le_value::<EncryptedDataBlobHeader>()?
            };

            if let Some(config) = config {
                let data = if magic == &ENCR_COMPR_BLOB_MAGIC_1_0 {
                    Self::decode_compressed_chunk(
                        config,
                        &self.raw_data[header_len..],
                        &head.iv,
                        &head.tag,
                    )?
                } else {
                    Self::decode_uncompressed_chunk(
                        config,
                        &self.raw_data[header_len..],
                        &head.iv,
                        &head.tag,
                    )?
                };
                if let Some(digest) = digest {
                    Self::verify_digest(&data, Some(config), digest)?;
                }
                Ok(data)
            } else {
                bail!("unable to decrypt blob - missing CryptConfig");
            }
        } else {
            bail!("Invalid blob magic number.");
        }
    }

    /// Load blob from ``reader``, verify CRC
    pub fn load_from_reader(reader: &mut dyn std::io::Read) -> Result<Self, Error> {
        let mut data = Vec::with_capacity(1024 * 1024);
        reader.read_to_end(&mut data)?;

        let blob = Self::from_raw(data)?;

        blob.verify_crc()?;

        Ok(blob)
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

    /// Returns if chunk is encrypted
    pub fn is_encrypted(&self) -> bool {
        let magic = self.magic();
        magic == &ENCR_COMPR_BLOB_MAGIC_1_0 || magic == &ENCRYPTED_BLOB_MAGIC_1_0
    }

    /// Returns if chunk is compressed
    pub fn is_compressed(&self) -> bool {
        let magic = self.magic();
        magic == &ENCR_COMPR_BLOB_MAGIC_1_0 || magic == &COMPRESSED_BLOB_MAGIC_1_0
    }

    /// Verify digest and data length for unencrypted chunks.
    ///
    /// To do that, we need to decompress data first. Please note that
    /// this is not possible for encrypted chunks. This function simply return Ok
    /// for encrypted chunks.
    /// Note: This does not call verify_crc, because this is usually done in load
    pub fn verify_unencrypted(
        &self,
        expected_chunk_size: usize,
        expected_digest: &[u8; 32],
    ) -> Result<(), Error> {
        let magic = self.magic();

        if magic == &ENCR_COMPR_BLOB_MAGIC_1_0 || magic == &ENCRYPTED_BLOB_MAGIC_1_0 {
            return Ok(());
        }

        // verifies digest!
        let data = self.decode(None, Some(expected_digest))?;

        if expected_chunk_size != data.len() {
            bail!(
                "detected chunk with wrong length ({} != {})",
                expected_chunk_size,
                data.len()
            );
        }

        Ok(())
    }

    fn verify_digest(
        data: &[u8],
        config: Option<&CryptConfig>,
        expected_digest: &[u8; 32],
    ) -> Result<(), Error> {
        let digest = match config {
            Some(config) => config.compute_digest(data),
            None => openssl::sha::sha256(data),
        };
        if &digest != expected_digest {
            bail!("detected chunk with wrong digest.");
        }

        Ok(())
    }

    /// Benchmark encryption speed
    pub fn encrypt_benchmark<W: Write>(
        config: &CryptConfig,
        data: &[u8],
        output: W,
    ) -> Result<(), Error> {
        let _ = Self::encrypt_to(config, data, output)?;
        Ok(())
    }

    // Encrypt data using a random 16 byte IV.
    //
    // Writes encrypted data to ``output``, Return the used IV and computed MAC.
    fn encrypt_to<W: Write>(
        config: &CryptConfig,
        data: &[u8],
        mut output: W,
    ) -> Result<([u8; 16], [u8; 16]), Error> {
        let mut iv = [0u8; 16];
        proxmox_sys::linux::fill_with_random_data(&mut iv)?;

        let mut tag = [0u8; 16];

        let mut c = config.data_crypter(&iv, Mode::Encrypt)?;

        const BUFFER_SIZE: usize = 32 * 1024;

        let mut encr_buf = [0u8; BUFFER_SIZE];
        let max_encoder_input = BUFFER_SIZE - config.cipher().block_size();

        let mut start = 0;
        loop {
            let mut end = start + max_encoder_input;
            if end > data.len() {
                end = data.len();
            }
            if end > start {
                let count = c.update(&data[start..end], &mut encr_buf)?;
                output.write_all(&encr_buf[..count])?;
                start = end;
            } else {
                break;
            }
        }

        let rest = c.finalize(&mut encr_buf)?;
        if rest > 0 {
            output.write_all(&encr_buf[..rest])?;
        }

        output.flush()?;

        c.get_tag(&mut tag)?;

        Ok((iv, tag))
    }

    // Decompress and decrypt data, verify MAC.
    fn decode_compressed_chunk(
        config: &CryptConfig,
        data: &[u8],
        iv: &[u8; 16],
        tag: &[u8; 16],
    ) -> Result<Vec<u8>, Error> {
        let dec = Vec::with_capacity(1024 * 1024);

        let mut decompressor = zstd::stream::write::Decoder::new(dec)?;

        let mut c = config.data_crypter(iv, Mode::Decrypt)?;

        const BUFFER_SIZE: usize = 32 * 1024;

        let mut decr_buf = [0u8; BUFFER_SIZE];
        let max_decoder_input = BUFFER_SIZE - config.cipher().block_size();

        let mut start = 0;
        loop {
            let mut end = start + max_decoder_input;
            if end > data.len() {
                end = data.len();
            }
            if end > start {
                let count = c.update(&data[start..end], &mut decr_buf)?;
                decompressor.write_all(&decr_buf[0..count])?;
                start = end;
            } else {
                break;
            }
        }

        c.set_tag(tag)?;
        let rest = c.finalize(&mut decr_buf)?;
        if rest > 0 {
            decompressor.write_all(&decr_buf[..rest])?;
        }

        decompressor.flush()?;

        Ok(decompressor.into_inner())
    }

    // Decrypt data, verify tag.
    fn decode_uncompressed_chunk(
        config: &CryptConfig,
        data: &[u8],
        iv: &[u8; 16],
        tag: &[u8; 16],
    ) -> Result<Vec<u8>, Error> {
        let decr_data = decrypt_aead(
            *config.cipher(),
            config.enc_key(),
            Some(iv),
            b"", //??
            data,
            tag,
        )?;

        Ok(decr_data)
    }
}

/// Builder for chunk DataBlobs
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

impl<'a, 'b> DataChunkBuilder<'a, 'b> {
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
    /// If set, chunks are encrypted
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

    /// Consume self and build the ``DataBlob``.
    ///
    /// Returns the blob and the computet digest.
    pub fn build(mut self) -> Result<(DataBlob, [u8; 32]), Error> {
        if !self.digest_computed {
            self.compute_digest();
        }

        let chunk = DataBlob::encode(self.orig_data, self.config, self.compress)?;
        Ok((chunk, self.digest))
    }

    /// Create a chunk filled with zeroes
    pub fn build_zero_chunk(
        crypt_config: Option<&CryptConfig>,
        chunk_size: usize,
        compress: bool,
    ) -> Result<(DataBlob, [u8; 32]), Error> {
        let zero_bytes = vec![0; chunk_size];
        let mut chunk_builder = DataChunkBuilder::new(&zero_bytes).compress(compress);
        if let Some(crypt_config) = crypt_config {
            chunk_builder = chunk_builder.crypt_config(crypt_config);
        }

        chunk_builder.build()
    }
}
