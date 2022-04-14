use anyhow::Error;
use std::io::Read;
use std::sync::Arc;

use proxmox_borrow::Tied;

use pbs_tools::crypt_config::CryptConfig;

pub struct ChecksumReader<R> {
    reader: R,
    hasher: crc32fast::Hasher,
    signer: Option<Tied<Arc<CryptConfig>, openssl::sign::Signer<'static>>>,
}

impl<R: Read> ChecksumReader<R> {
    pub fn new(reader: R, config: Option<Arc<CryptConfig>>) -> Self {
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
            reader,
            hasher,
            signer,
        }
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

impl<R: Read> Read for ChecksumReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, std::io::Error> {
        let count = self.reader.read(buf)?;
        if count > 0 {
            self.hasher.update(&buf[..count]);
            if let Some(ref mut signer) = self.signer {
                signer.update(&buf[..count]).map_err(|err| {
                    std::io::Error::new(
                        std::io::ErrorKind::Other,
                        format!("hmac update failed - {}", err),
                    )
                })?;
            }
        }
        Ok(count)
    }
}
