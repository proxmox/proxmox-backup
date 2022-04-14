use std::io::Write;
use std::sync::Arc;

use anyhow::Error;

use proxmox_borrow::Tied;

use pbs_tools::crypt_config::CryptConfig;

pub struct ChecksumWriter<W> {
    writer: W,
    hasher: crc32fast::Hasher,
    signer: Option<Tied<Arc<CryptConfig>, openssl::sign::Signer<'static>>>,
}

impl<W: Write> ChecksumWriter<W> {
    pub fn new(writer: W, config: Option<Arc<CryptConfig>>) -> Self {
        let hasher = crc32fast::Hasher::new();
        let signer = match config {
            Some(config) => {
                let tied_signer = Tied::new(config, |config| {
                    Box::new(unsafe { (*config).data_signer() })
                });
                Some(tied_signer)
            }
            None => None,
        };
        Self {
            writer,
            hasher,
            signer,
        }
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

impl<W: Write> Write for ChecksumWriter<W> {
    fn write(&mut self, buf: &[u8]) -> Result<usize, std::io::Error> {
        self.hasher.update(buf);
        if let Some(ref mut signer) = self.signer {
            signer.update(buf).map_err(|err| {
                std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("hmac update failed - {}", err),
                )
            })?;
        }
        self.writer.write(buf)
    }

    fn flush(&mut self) -> Result<(), std::io::Error> {
        self.writer.flush()
    }
}
