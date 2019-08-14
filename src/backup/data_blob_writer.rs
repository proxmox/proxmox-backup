use failure::*;
use std::io::{Write, Seek, SeekFrom};
use proxmox::tools::io::WriteExt;

use super::*;

enum BlobWriterState<'a, W: Write> {
    Uncompressed { csum_writer: ChecksumWriter<'a, W> },
    Compressed { compr: zstd::stream::write::Encoder<ChecksumWriter<'a, W>> },
    Signed { csum_writer: ChecksumWriter<'a, W> },
    SignedCompressed { compr: zstd::stream::write::Encoder<ChecksumWriter<'a, W>> },
    Encrypted { crypt_writer: CryptWriter<ChecksumWriter<'a, W>> },
    EncryptedCompressed { compr: zstd::stream::write::Encoder<CryptWriter<ChecksumWriter<'a, W>>> },
}

/// Data blob writer
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
