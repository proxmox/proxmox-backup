use failure::*;
use std::io::Read;

pub struct ChecksumReader<'a, R> {
    reader: R,
    hasher: crc32fast::Hasher,
    signer: Option<openssl::sign::Signer<'a>>,
}

impl <'a, R: Read> ChecksumReader<'a, R> {

    pub fn new(reader: R, signer: Option<openssl::sign::Signer<'a>>) -> Self {
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
