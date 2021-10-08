use std::io::{Seek, SeekFrom, Write};
use std::sync::Arc;

use anyhow::Error;

use proxmox_io::WriteExt;

use pbs_tools::crypt_config::CryptConfig;

use crate::checksum_writer::ChecksumWriter;
use crate::crypt_writer::CryptWriter;
use crate::file_formats::{self, DataBlobHeader, EncryptedDataBlobHeader};

enum BlobWriterState<'writer, W: Write> {
    Uncompressed {
        csum_writer: ChecksumWriter<W>,
    },
    Compressed {
        compr: zstd::stream::write::Encoder<'writer, ChecksumWriter<W>>,
    },
    Encrypted {
        crypt_writer: CryptWriter<ChecksumWriter<W>>,
    },
    EncryptedCompressed {
        compr: zstd::stream::write::Encoder<'writer, CryptWriter<ChecksumWriter<W>>>,
    },
}

/// Data blob writer
pub struct DataBlobWriter<'writer, W: Write> {
    state: BlobWriterState<'writer, W>,
}

impl<W: Write + Seek> DataBlobWriter<'_, W> {
    pub fn new_uncompressed(mut writer: W) -> Result<Self, Error> {
        writer.seek(SeekFrom::Start(0))?;
        let head = DataBlobHeader {
            magic: file_formats::UNCOMPRESSED_BLOB_MAGIC_1_0,
            crc: [0; 4],
        };
        unsafe {
            writer.write_le_value(head)?;
        }
        let csum_writer = ChecksumWriter::new(writer, None);
        Ok(Self {
            state: BlobWriterState::Uncompressed { csum_writer },
        })
    }

    pub fn new_compressed(mut writer: W) -> Result<Self, Error> {
        writer.seek(SeekFrom::Start(0))?;
        let head = DataBlobHeader {
            magic: file_formats::COMPRESSED_BLOB_MAGIC_1_0,
            crc: [0; 4],
        };
        unsafe {
            writer.write_le_value(head)?;
        }
        let csum_writer = ChecksumWriter::new(writer, None);
        let compr = zstd::stream::write::Encoder::new(csum_writer, 1)?;
        Ok(Self {
            state: BlobWriterState::Compressed { compr },
        })
    }

    pub fn new_encrypted(mut writer: W, config: Arc<CryptConfig>) -> Result<Self, Error> {
        writer.seek(SeekFrom::Start(0))?;
        let head = EncryptedDataBlobHeader {
            head: DataBlobHeader {
                magic: file_formats::ENCRYPTED_BLOB_MAGIC_1_0,
                crc: [0; 4],
            },
            iv: [0u8; 16],
            tag: [0u8; 16],
        };
        unsafe {
            writer.write_le_value(head)?;
        }

        let csum_writer = ChecksumWriter::new(writer, None);
        let crypt_writer = CryptWriter::new(csum_writer, config)?;
        Ok(Self {
            state: BlobWriterState::Encrypted { crypt_writer },
        })
    }

    pub fn new_encrypted_compressed(
        mut writer: W,
        config: Arc<CryptConfig>,
    ) -> Result<Self, Error> {
        writer.seek(SeekFrom::Start(0))?;
        let head = EncryptedDataBlobHeader {
            head: DataBlobHeader {
                magic: file_formats::ENCR_COMPR_BLOB_MAGIC_1_0,
                crc: [0; 4],
            },
            iv: [0u8; 16],
            tag: [0u8; 16],
        };
        unsafe {
            writer.write_le_value(head)?;
        }

        let csum_writer = ChecksumWriter::new(writer, None);
        let crypt_writer = CryptWriter::new(csum_writer, config)?;
        let compr = zstd::stream::write::Encoder::new(crypt_writer, 1)?;
        Ok(Self {
            state: BlobWriterState::EncryptedCompressed { compr },
        })
    }

    pub fn finish(self) -> Result<W, Error> {
        match self.state {
            BlobWriterState::Uncompressed { csum_writer } => {
                // write CRC
                let (mut writer, crc, _) = csum_writer.finish()?;
                let head = DataBlobHeader {
                    magic: file_formats::UNCOMPRESSED_BLOB_MAGIC_1_0,
                    crc: crc.to_le_bytes(),
                };

                writer.seek(SeekFrom::Start(0))?;
                unsafe {
                    writer.write_le_value(head)?;
                }

                Ok(writer)
            }
            BlobWriterState::Compressed { compr } => {
                let csum_writer = compr.finish()?;
                let (mut writer, crc, _) = csum_writer.finish()?;

                let head = DataBlobHeader {
                    magic: file_formats::COMPRESSED_BLOB_MAGIC_1_0,
                    crc: crc.to_le_bytes(),
                };

                writer.seek(SeekFrom::Start(0))?;
                unsafe {
                    writer.write_le_value(head)?;
                }

                Ok(writer)
            }
            BlobWriterState::Encrypted { crypt_writer } => {
                let (csum_writer, iv, tag) = crypt_writer.finish()?;
                let (mut writer, crc, _) = csum_writer.finish()?;

                let head = EncryptedDataBlobHeader {
                    head: DataBlobHeader {
                        magic: file_formats::ENCRYPTED_BLOB_MAGIC_1_0,
                        crc: crc.to_le_bytes(),
                    },
                    iv,
                    tag,
                };
                writer.seek(SeekFrom::Start(0))?;
                unsafe {
                    writer.write_le_value(head)?;
                }
                Ok(writer)
            }
            BlobWriterState::EncryptedCompressed { compr } => {
                let crypt_writer = compr.finish()?;
                let (csum_writer, iv, tag) = crypt_writer.finish()?;
                let (mut writer, crc, _) = csum_writer.finish()?;

                let head = EncryptedDataBlobHeader {
                    head: DataBlobHeader {
                        magic: file_formats::ENCR_COMPR_BLOB_MAGIC_1_0,
                        crc: crc.to_le_bytes(),
                    },
                    iv,
                    tag,
                };
                writer.seek(SeekFrom::Start(0))?;
                unsafe {
                    writer.write_le_value(head)?;
                }
                Ok(writer)
            }
        }
    }
}

impl<W: Write + Seek> Write for DataBlobWriter<'_, W> {
    fn write(&mut self, buf: &[u8]) -> Result<usize, std::io::Error> {
        match self.state {
            BlobWriterState::Uncompressed {
                ref mut csum_writer,
            } => csum_writer.write(buf),
            BlobWriterState::Compressed { ref mut compr } => compr.write(buf),
            BlobWriterState::Encrypted {
                ref mut crypt_writer,
            } => crypt_writer.write(buf),
            BlobWriterState::EncryptedCompressed { ref mut compr } => compr.write(buf),
        }
    }

    fn flush(&mut self) -> Result<(), std::io::Error> {
        match self.state {
            BlobWriterState::Uncompressed {
                ref mut csum_writer,
            } => csum_writer.flush(),
            BlobWriterState::Compressed { ref mut compr } => compr.flush(),
            BlobWriterState::Encrypted {
                ref mut crypt_writer,
            } => crypt_writer.flush(),
            BlobWriterState::EncryptedCompressed { ref mut compr } => compr.flush(),
        }
    }
}
