use std::io::Write;
use std::sync::Arc;

use anyhow::Error;

use pbs_tools::crypt_config::CryptConfig;

pub struct CryptWriter<W> {
    writer: W,
    block_size: usize,
    encr_buf: Box<[u8; 64 * 1024]>,
    iv: [u8; 16],
    crypter: openssl::symm::Crypter,
}

impl<W: Write> CryptWriter<W> {
    pub fn new(writer: W, config: Arc<CryptConfig>) -> Result<Self, Error> {
        let mut iv = [0u8; 16];
        proxmox_sys::linux::fill_with_random_data(&mut iv)?;
        let block_size = config.cipher().block_size();

        let crypter = config.data_crypter(&iv, openssl::symm::Mode::Encrypt)?;

        Ok(Self {
            writer,
            iv,
            crypter,
            block_size,
            encr_buf: Box::new([0u8; 64 * 1024]),
        })
    }

    pub fn finish(mut self) -> Result<(W, [u8; 16], [u8; 16]), Error> {
        let rest = self.crypter.finalize(self.encr_buf.as_mut())?;
        if rest > 0 {
            self.writer.write_all(&self.encr_buf[..rest])?;
        }

        self.writer.flush()?;

        let mut tag = [0u8; 16];
        self.crypter.get_tag(&mut tag)?;

        Ok((self.writer, self.iv, tag))
    }
}

impl<W: Write> Write for CryptWriter<W> {
    fn write(&mut self, buf: &[u8]) -> Result<usize, std::io::Error> {
        let mut write_size = buf.len();
        if write_size > (self.encr_buf.len() - self.block_size) {
            write_size = self.encr_buf.len() - self.block_size;
        }
        let count = self
            .crypter
            .update(&buf[..write_size], self.encr_buf.as_mut())
            .map_err(|err| {
                std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("crypter update failed - {}", err),
                )
            })?;

        self.writer.write_all(&self.encr_buf[..count])?;

        Ok(write_size)
    }

    fn flush(&mut self) -> Result<(), std::io::Error> {
        Ok(())
    }
}
