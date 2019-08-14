use failure::*;
use std::io::Write;

pub struct ChecksumWriter<'a, W> {
    writer: W,
    hasher: crc32fast::Hasher,
    signer: Option<openssl::sign::Signer<'a>>,
}

impl <'a, W: Write> ChecksumWriter<'a, W> {

    pub fn new(writer: W, signer: Option<openssl::sign::Signer<'a>>) -> Self {
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
