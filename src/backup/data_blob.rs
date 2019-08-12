use failure::*;
use std::convert::TryInto;

use proxmox::tools::io::{ReadExt, WriteExt};

const MAX_BLOB_SIZE: usize = 128*1024*1024;

use super::*;

/// Data blob binary storage format
///
/// Data blobs store arbitrary binary data (< 128MB), and can be
/// compressed and encrypted. A simply binary format is used to store
/// them on disk or transfer them over the network. Please use index
/// files to store large data files (".fidx" of ".didx").
///
pub struct DataBlob {
    raw_data: Vec<u8>, // tagged, compressed, encryped data
}

impl DataBlob {

    pub fn header_size(magic: &[u8; 8]) -> usize {
        match magic {
            &UNCOMPRESSED_CHUNK_MAGIC_1_0 => std::mem::size_of::<DataChunkHeader>(),
            &COMPRESSED_CHUNK_MAGIC_1_0 => std::mem::size_of::<DataChunkHeader>(),
            &ENCRYPTED_CHUNK_MAGIC_1_0 => std::mem::size_of::<EncryptedDataChunkHeader>(),
            &ENCR_COMPR_CHUNK_MAGIC_1_0 => std::mem::size_of::<EncryptedDataChunkHeader>(),

            &UNCOMPRESSED_BLOB_MAGIC_1_0 => std::mem::size_of::<DataBlobHeader>(),
            &COMPRESSED_BLOB_MAGIC_1_0 => std::mem::size_of::<DataBlobHeader>(),
            &ENCRYPTED_BLOB_MAGIC_1_0 => std::mem::size_of::<EncryptedDataBlobHeader>(),
            &ENCR_COMPR_BLOB_MAGIC_1_0 => std::mem::size_of::<EncryptedDataBlobHeader>(),
            &AUTHENTICATED_BLOB_MAGIC_1_0 => std::mem::size_of::<AuthenticatedDataBlobHeader>(),
            &AUTH_COMPR_BLOB_MAGIC_1_0 => std::mem::size_of::<AuthenticatedDataBlobHeader>(),
            _ => panic!("unknown blob magic"),
        }
    }

    /// accessor to raw_data field
    pub fn raw_data(&self) -> &[u8]  {
        &self.raw_data
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
        let crc_o = proxmox::tools::offsetof!(DataBlobHeader, crc);
        u32::from_le_bytes(self.raw_data[crc_o..crc_o+4].try_into().unwrap())
    }

    // set the CRC checksum field
    pub fn set_crc(&mut self, crc: u32) {
        let crc_o = proxmox::tools::offsetof!(DataBlobHeader, crc);
        self.raw_data[crc_o..crc_o+4].copy_from_slice(&crc.to_le_bytes());
    }

    /// compute the CRC32 checksum
    pub fn compute_crc(&self) -> u32 {
        let mut hasher = crc32fast::Hasher::new();
        let start = Self::header_size(self.magic()); // start after HEAD
        hasher.update(&self.raw_data[start..]);
        hasher.finalize()
    }

    /// verify the CRC32 checksum
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
            unsafe {
                raw_data.write_le_value(dummy_head)?;
            }

            let (iv, tag) = config.encrypt_to(data, &mut raw_data)?;

            let head = EncryptedDataBlobHeader {
                head: DataBlobHeader { magic, crc: [0; 4] }, iv, tag,
            };

            unsafe {
                (&mut raw_data[0..header_len]).write_le_value(head)?;
            }

            DataBlob { raw_data }
        } else {

            let max_data_len = data.len() + std::mem::size_of::<DataBlobHeader>();
            if compress {
                let mut comp_data = Vec::with_capacity(max_data_len);

                let head =  DataBlobHeader {
                    magic: COMPRESSED_BLOB_MAGIC_1_0,
                    crc: [0; 4],
                };
                unsafe {
                    comp_data.write_le_value(head)?;
                }

                zstd::stream::copy_encode(data, &mut comp_data, 1)?;

                if comp_data.len() < max_data_len {
                    let mut blob = DataBlob { raw_data: comp_data };
                    blob.set_crc(blob.compute_crc());
                    return Ok(blob);
                }
            }

            let mut raw_data = Vec::with_capacity(max_data_len);

            let head =  DataBlobHeader {
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

    /// Decode blob data
    pub fn decode(self, config: Option<&CryptConfig>) -> Result<Vec<u8>, Error> {

        let magic = self.magic();

        if magic == &UNCOMPRESSED_BLOB_MAGIC_1_0 {
            let data_start = std::mem::size_of::<DataBlobHeader>();
            return Ok(self.raw_data[data_start..].to_vec());
        } else if magic == &COMPRESSED_BLOB_MAGIC_1_0 {
            let data_start = std::mem::size_of::<DataBlobHeader>();
            let data = zstd::block::decompress(&self.raw_data[data_start..], MAX_BLOB_SIZE)?;
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
        } else if magic == &AUTH_COMPR_BLOB_MAGIC_1_0 || magic == &AUTHENTICATED_BLOB_MAGIC_1_0 {
            let header_len = std::mem::size_of::<AuthenticatedDataBlobHeader>();
            let head = unsafe {
                (&self.raw_data[..header_len]).read_le_value::<AuthenticatedDataBlobHeader>()?
            };

            let data_start = std::mem::size_of::<AuthenticatedDataBlobHeader>();

            // Note: only verify if we have a crypt config
            if let Some(config) = config  {
                let signature = config.compute_auth_tag(&self.raw_data[data_start..]);
                if signature != head.tag {
                    bail!("verifying blob signature failed");
                }
            }

            if magic == &AUTH_COMPR_BLOB_MAGIC_1_0 {
                let data = zstd::block::decompress(&self.raw_data[data_start..], 16*1024*1024)?;
                return Ok(data);
            } else {
                return Ok(self.raw_data[data_start..].to_vec());
            }
        } else {
            bail!("Invalid blob magic number.");
        }
    }

    /// Create a signed DataBlob, optionally compressed
    pub fn create_signed(
        data: &[u8],
        config: &CryptConfig,
        compress: bool,
    ) -> Result<Self, Error> {

        if data.len() > MAX_BLOB_SIZE {
            bail!("data blob too large ({} bytes).", data.len());
        }

        let compr_data;
        let (_compress, data, magic) = if compress {
            compr_data = zstd::block::compress(data, 1)?;
            // Note: We only use compression if result is shorter
            if compr_data.len() < data.len() {
                (true, &compr_data[..], AUTH_COMPR_BLOB_MAGIC_1_0)
            } else {
                (false, data, AUTHENTICATED_BLOB_MAGIC_1_0)
            }
        } else {
            (false, data, AUTHENTICATED_BLOB_MAGIC_1_0)
        };

        let header_len = std::mem::size_of::<AuthenticatedDataBlobHeader>();
        let mut raw_data = Vec::with_capacity(data.len() + header_len);

        let head = AuthenticatedDataBlobHeader {
            head: DataBlobHeader { magic, crc: [0; 4] },
            tag: config.compute_auth_tag(data),
        };
        unsafe {
            raw_data.write_le_value(head)?;
        }
        raw_data.extend_from_slice(data);

        let mut blob = DataBlob { raw_data };
        blob.set_crc(blob.compute_crc());

        return Ok(blob);
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
        } else if magic == AUTH_COMPR_BLOB_MAGIC_1_0 || magic == AUTHENTICATED_BLOB_MAGIC_1_0 {
            if data.len() < std::mem::size_of::<AuthenticatedDataBlobHeader>() {
                bail!("authenticated blob too small ({} bytes).", data.len());
            }

            let blob = DataBlob { raw_data: data };

            Ok(blob)
        } else {
            bail!("unable to parse raw blob - wrong magic");
        }
    }

}

use std::io::{Read, BufRead, BufReader, Write, Seek, SeekFrom};

struct CryptWriter<W> {
    writer: W,
    encr_buf: [u8; 64*1024],
    iv: [u8; 16],
    crypter: openssl::symm::Crypter,
}

impl <W: Write> CryptWriter<W> {

    fn new(writer: W, config: &CryptConfig) -> Result<Self, Error> {
        let mut iv = [0u8; 16];
        proxmox::sys::linux::fill_with_random_data(&mut iv)?;

        let crypter = config.data_crypter(&iv, openssl::symm::Mode::Encrypt)?;

        Ok(Self { writer, iv, crypter, encr_buf: [0u8; 64*1024] })
    }

    fn finish(mut self) ->  Result<(W, [u8; 16], [u8; 16]), Error> {
        let rest = self.crypter.finalize(&mut self.encr_buf)?;
        if rest > 0 {
            self.writer.write_all(&self.encr_buf[..rest])?;
        }

        self.writer.flush()?;

        let mut tag = [0u8; 16];
        self.crypter.get_tag(&mut tag)?;

        Ok((self.writer, self.iv, tag))
    }
}

impl <W: Write> Write for CryptWriter<W> {

    fn write(&mut self, buf: &[u8]) -> Result<usize, std::io::Error> {
        let count = self.crypter.update(buf, &mut self.encr_buf)
            .map_err(|err| {
                std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("crypter update failed - {}", err))
            })?;

        self.writer.write_all(&self.encr_buf[..count])?;

        Ok(count)
    }

    fn flush(&mut self) -> Result<(), std::io::Error> {
        Ok(())
    }
}

struct ChecksumWriter<'a, W> {
    writer: W,
    hasher: crc32fast::Hasher,
    signer: Option<openssl::sign::Signer<'a>>,
}

impl <'a, W: Write> ChecksumWriter<'a, W> {

    fn new(writer: W, signer: Option<openssl::sign::Signer<'a>>) -> Self {
        let hasher = crc32fast::Hasher::new();
        Self { writer, hasher, signer }
    }

    pub fn finish(mut self) -> Result<(W, u32, Option<[u8; 32]>), Error> {
        let crc = self.hasher.finalize();

        if let Some(ref mut signer) = self.signer {
            let mut tag = [0u8; 32];
            signer.sign(&mut tag)?;
            Ok((self.writer, crc, Some(tag)))
        } else {
            Ok((self.writer, crc, None))
        }
    }
}

impl <'a, W: Write> Write for ChecksumWriter<'a, W> {

    fn write(&mut self, buf: &[u8]) -> Result<usize, std::io::Error> {
        self.hasher.update(buf);
        if let Some(ref mut signer) = self.signer {
            signer.update(buf)
                .map_err(|err| {
                    std::io::Error::new(
                        std::io::ErrorKind::Other,
                        format!("hmac update failed - {}", err))
                })?;
        }
        self.writer.write(buf)
    }

    fn flush(&mut self) -> Result<(), std::io::Error> {
        self.writer.flush()
    }
}

enum BlobWriterState<'a, W: Write> {
    Uncompressed { csum_writer: ChecksumWriter<'a, W> },
    Compressed { compr: zstd::stream::write::Encoder<ChecksumWriter<'a, W>> },
    Signed { csum_writer: ChecksumWriter<'a, W> },
    SignedCompressed { compr: zstd::stream::write::Encoder<ChecksumWriter<'a, W>> },
    Encrypted { crypt_writer: CryptWriter<ChecksumWriter<'a, W>> },
    EncryptedCompressed { compr: zstd::stream::write::Encoder<CryptWriter<ChecksumWriter<'a, W>>> },
}

/// Write compressed data blobs
pub struct DataBlobWriter<'a, W: Write> {
    state: BlobWriterState<'a, W>,
}

impl <'a, W: Write + Seek> DataBlobWriter<'a, W> {

    pub fn new_uncompressed(mut writer: W) -> Result<Self, Error> {
        writer.seek(SeekFrom::Start(0))?;
        let head = DataBlobHeader { magic: UNCOMPRESSED_BLOB_MAGIC_1_0, crc: [0; 4] };
        unsafe {
            writer.write_le_value(head)?;
        }
        let csum_writer = ChecksumWriter::new(writer, None);
        Ok(Self { state: BlobWriterState::Uncompressed { csum_writer }})
    }

    pub fn new_compressed(mut writer: W) -> Result<Self, Error> {
         writer.seek(SeekFrom::Start(0))?;
        let head = DataBlobHeader { magic: COMPRESSED_BLOB_MAGIC_1_0, crc: [0; 4] };
        unsafe {
            writer.write_le_value(head)?;
        }
        let csum_writer = ChecksumWriter::new(writer, None);
        let compr = zstd::stream::write::Encoder::new(csum_writer, 1)?;
        Ok(Self { state: BlobWriterState::Compressed { compr }})
    }

    pub fn new_signed(mut writer: W, config: &'a CryptConfig) -> Result<Self, Error> {
        writer.seek(SeekFrom::Start(0))?;
        let head = AuthenticatedDataBlobHeader {
            head: DataBlobHeader { magic: AUTHENTICATED_BLOB_MAGIC_1_0, crc: [0; 4] },
            tag: [0u8; 32],
        };
        unsafe {
            writer.write_le_value(head)?;
        }
        let signer = config.data_signer();
        let csum_writer = ChecksumWriter::new(writer, Some(signer));
        Ok(Self { state:  BlobWriterState::Signed { csum_writer }})
    }

    pub fn new_signed_compressed(mut writer: W, config: &'a CryptConfig) -> Result<Self, Error> {
        writer.seek(SeekFrom::Start(0))?;
        let head = AuthenticatedDataBlobHeader {
            head: DataBlobHeader { magic: AUTH_COMPR_BLOB_MAGIC_1_0, crc: [0; 4] },
            tag: [0u8; 32],
        };
        unsafe {
            writer.write_le_value(head)?;
        }
        let signer = config.data_signer();
        let csum_writer = ChecksumWriter::new(writer, Some(signer));
        let compr = zstd::stream::write::Encoder::new(csum_writer, 1)?;
        Ok(Self { state: BlobWriterState::SignedCompressed { compr }})
    }

    pub fn new_encrypted(mut writer: W, config: &'a CryptConfig) -> Result<Self, Error> {
        writer.seek(SeekFrom::Start(0))?;
        let head = EncryptedDataBlobHeader {
            head: DataBlobHeader { magic: ENCRYPTED_BLOB_MAGIC_1_0, crc: [0; 4] },
            iv: [0u8; 16],
            tag: [0u8; 16],
        };
        unsafe {
            writer.write_le_value(head)?;
        }

        let csum_writer = ChecksumWriter::new(writer, None);
        let crypt_writer =  CryptWriter::new(csum_writer, config)?;
        Ok(Self { state: BlobWriterState::Encrypted { crypt_writer }})
    }

    pub fn new_encrypted_compressed(mut writer: W, config: &'a CryptConfig) -> Result<Self, Error> {
        writer.seek(SeekFrom::Start(0))?;
        let head = EncryptedDataBlobHeader {
            head: DataBlobHeader { magic: ENCR_COMPR_BLOB_MAGIC_1_0, crc: [0; 4] },
            iv: [0u8; 16],
            tag: [0u8; 16],
        };
        unsafe {
            writer.write_le_value(head)?;
        }

        let csum_writer = ChecksumWriter::new(writer, None);
        let crypt_writer =  CryptWriter::new(csum_writer, config)?;
        let compr = zstd::stream::write::Encoder::new(crypt_writer, 1)?;
        Ok(Self { state: BlobWriterState::EncryptedCompressed { compr }})
    }

    pub fn finish(self) -> Result<W, Error> {
        match self.state {
            BlobWriterState::Uncompressed { csum_writer } => {
                // write CRC
                let (mut writer, crc, _) = csum_writer.finish()?;
                let head = DataBlobHeader { magic: UNCOMPRESSED_BLOB_MAGIC_1_0, crc: crc.to_le_bytes() };

                writer.seek(SeekFrom::Start(0))?;
                unsafe {
                    writer.write_le_value(head)?;
                }

                return Ok(writer)
            }
            BlobWriterState::Compressed { compr } => {
                let csum_writer = compr.finish()?;
                let (mut writer, crc, _) = csum_writer.finish()?;

                let head = DataBlobHeader { magic: COMPRESSED_BLOB_MAGIC_1_0, crc: crc.to_le_bytes() };

                writer.seek(SeekFrom::Start(0))?;
                unsafe {
                    writer.write_le_value(head)?;
                }

                return Ok(writer)
            }
            BlobWriterState::Signed { csum_writer } => {
                let (mut writer, crc, tag) = csum_writer.finish()?;

                let head = AuthenticatedDataBlobHeader {
                    head: DataBlobHeader { magic: AUTHENTICATED_BLOB_MAGIC_1_0, crc: crc.to_le_bytes() },
                    tag: tag.unwrap(),
                };

                writer.seek(SeekFrom::Start(0))?;
                unsafe {
                    writer.write_le_value(head)?;
                }

                return Ok(writer)
            }
            BlobWriterState::SignedCompressed { compr } => {
                let csum_writer = compr.finish()?;
                let (mut writer, crc, tag) = csum_writer.finish()?;

                let head = AuthenticatedDataBlobHeader {
                    head: DataBlobHeader { magic: AUTH_COMPR_BLOB_MAGIC_1_0, crc: crc.to_le_bytes() },
                    tag: tag.unwrap(),
                };

                writer.seek(SeekFrom::Start(0))?;
                unsafe {
                    writer.write_le_value(head)?;
                }

                return Ok(writer)
            }
            BlobWriterState::Encrypted { crypt_writer } => {
                let (csum_writer, iv, tag) = crypt_writer.finish()?;
                let (mut writer, crc, _) = csum_writer.finish()?;

                let head = EncryptedDataBlobHeader {
                    head: DataBlobHeader { magic: ENCRYPTED_BLOB_MAGIC_1_0, crc: crc.to_le_bytes() },
                    iv, tag,
                };
                writer.seek(SeekFrom::Start(0))?;
                unsafe {
                    writer.write_le_value(head)?;
                }
                return Ok(writer)
            }
            BlobWriterState::EncryptedCompressed { compr } => {
                let crypt_writer = compr.finish()?;
                let (csum_writer, iv, tag) = crypt_writer.finish()?;
                let (mut writer, crc, _) = csum_writer.finish()?;

                let head = EncryptedDataBlobHeader {
                    head: DataBlobHeader { magic: ENCR_COMPR_BLOB_MAGIC_1_0, crc: crc.to_le_bytes() },
                    iv, tag,
                };
                writer.seek(SeekFrom::Start(0))?;
                unsafe {
                    writer.write_le_value(head)?;
                }
                return Ok(writer)
            }
        }
    }
}

impl <'a, W: Write + Seek> Write for DataBlobWriter<'a, W> {

    fn write(&mut self, buf: &[u8]) -> Result<usize, std::io::Error> {
        match self.state {
            BlobWriterState::Uncompressed { ref mut csum_writer } => {
                csum_writer.write(buf)
            }
            BlobWriterState::Compressed { ref mut compr } => {
                compr.write(buf)
            }
            BlobWriterState::Signed { ref mut csum_writer } => {
                csum_writer.write(buf)
            }
            BlobWriterState::SignedCompressed { ref mut compr } => {
               compr.write(buf)
            }
            BlobWriterState::Encrypted { ref mut crypt_writer } => {
                crypt_writer.write(buf)
            }
            BlobWriterState::EncryptedCompressed { ref mut compr } => {
                compr.write(buf)
            }
        }
    }

    fn flush(&mut self) -> Result<(), std::io::Error> {
        match self.state {
            BlobWriterState::Uncompressed { ref mut csum_writer } => {
                csum_writer.flush()
            }
            BlobWriterState::Compressed { ref mut compr } => {
                compr.flush()
            }
            BlobWriterState::Signed { ref mut csum_writer } => {
                csum_writer.flush()
            }
            BlobWriterState::SignedCompressed { ref mut compr } => {
                compr.flush()
            }
            BlobWriterState::Encrypted { ref mut crypt_writer } => {
               crypt_writer.flush()
            }
            BlobWriterState::EncryptedCompressed { ref mut compr } => {
                compr.flush()
            }
        }
    }
}

struct ChecksumReader<'a, R> {
    reader: R,
    hasher: crc32fast::Hasher,
    signer: Option<openssl::sign::Signer<'a>>,
}

impl <'a, R: Read> ChecksumReader<'a, R> {

    fn new(reader: R, signer: Option<openssl::sign::Signer<'a>>) -> Self {
        let hasher = crc32fast::Hasher::new();
        Self { reader, hasher, signer }
    }

    pub fn finish(mut self) -> Result<(R, u32, Option<[u8; 32]>), Error> {
        let crc = self.hasher.finalize();

        if let Some(ref mut signer) = self.signer {
            let mut tag = [0u8; 32];
            signer.sign(&mut tag)?;
            Ok((self.reader, crc, Some(tag)))
        } else {
            Ok((self.reader, crc, None))
        }
    }
}

impl <'a, R: Read> Read for ChecksumReader<'a, R> {

    fn read(&mut self, buf: &mut [u8]) -> Result<usize, std::io::Error> {
        let count = self.reader.read(buf)?;
        if count > 0 {
            self.hasher.update(&buf[..count]);
            if let Some(ref mut signer) = self.signer {
                signer.update(&buf[..count])
                    .map_err(|err| {
                        std::io::Error::new(
                            std::io::ErrorKind::Other,
                            format!("hmac update failed - {}", err))
                    })?;
            }
        }
        Ok(count)
    }
}

enum BlobReaderState<'a, R: Read> {
    Uncompressed { expected_crc: u32, csum_reader: ChecksumReader<'a, R> },
    Compressed { expected_crc: u32, decompr: zstd::stream::read::Decoder<BufReader<ChecksumReader<'a, R>>> },
}

/// Read data blobs
pub struct DataBlobReader<'a, R: Read> {
    state: BlobReaderState<'a, R>,
}

impl <'a, R: Read> DataBlobReader<'a, R> {

    pub fn new(mut reader: R) -> Result<Self, Error> {

        let head: DataBlobHeader = unsafe { reader.read_le_value()? };
        match head.magic {
            UNCOMPRESSED_BLOB_MAGIC_1_0 => {
                let expected_crc = u32::from_le_bytes(head.crc);
                let csum_reader =  ChecksumReader::new(reader, None);
                Ok(Self { state: BlobReaderState::Uncompressed { expected_crc, csum_reader }})
            }
            COMPRESSED_BLOB_MAGIC_1_0 => {
                let expected_crc = u32::from_le_bytes(head.crc);
                let csum_reader =  ChecksumReader::new(reader, None);

                let decompr = zstd::stream::read::Decoder::new(csum_reader)?;
                Ok(Self { state: BlobReaderState::Compressed { expected_crc, decompr }})
            }
            _ => bail!("got wrong magic number {:?}", head.magic)
        }
    }

    pub fn finish(self) -> Result<R, Error> {
       match self.state {
           BlobReaderState::Uncompressed { csum_reader, expected_crc } => {
               let (reader, crc, _) = csum_reader.finish()?;
               if crc != expected_crc {
                   bail!("blob crc check failed");
               }
               Ok(reader)
           }
           BlobReaderState::Compressed { expected_crc, decompr } => {
               let csum_reader = decompr.finish().into_inner();
               let (reader, crc, _) = csum_reader.finish()?;
               if crc != expected_crc {
                   bail!("blob crc check failed");
               }
               Ok(reader)
           }
       }
    }
}

impl <'a, R: BufRead> Read for DataBlobReader<'a, R> {

    fn read(&mut self, buf: &mut [u8]) -> Result<usize, std::io::Error> {
        match &mut self.state {
            BlobReaderState::Uncompressed { csum_reader, .. } => {
                csum_reader.read(buf)
            }
            BlobReaderState::Compressed { decompr, .. } => {
                decompr.read(buf)
            }
        }
    }
}
