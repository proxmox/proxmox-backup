use std::io::{BufRead, Read};
use std::sync::Arc;

use anyhow::{bail, Error};

use pbs_tools::crypt_config::CryptConfig;

pub struct CryptReader<R> {
    reader: R,
    small_read_buf: Vec<u8>,
    block_size: usize,
    crypter: openssl::symm::Crypter,
    finalized: bool,
}

impl<R: BufRead> CryptReader<R> {
    pub fn new(
        reader: R,
        iv: [u8; 16],
        tag: [u8; 16],
        config: Arc<CryptConfig>,
    ) -> Result<Self, Error> {
        let block_size = config.cipher().block_size(); // Note: block size is normally 1 byte for stream ciphers
        if block_size.count_ones() != 1 || block_size > 512 {
            bail!("unexpected Cipher block size {}", block_size);
        }
        let mut crypter = config.data_crypter(&iv, openssl::symm::Mode::Decrypt)?;
        crypter.set_tag(&tag)?;

        Ok(Self {
            reader,
            crypter,
            block_size,
            finalized: false,
            small_read_buf: Vec::new(),
        })
    }

    pub fn finish(self) -> Result<R, Error> {
        if !self.finalized {
            bail!("CryptReader not successfully finalized.");
        }
        Ok(self.reader)
    }
}

impl<R: BufRead> Read for CryptReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, std::io::Error> {
        if !self.small_read_buf.is_empty() {
            let max = if self.small_read_buf.len() > buf.len() {
                buf.len()
            } else {
                self.small_read_buf.len()
            };
            let rest = self.small_read_buf.split_off(max);
            buf[..max].copy_from_slice(&self.small_read_buf);
            self.small_read_buf = rest;
            return Ok(max);
        }

        let data = self.reader.fill_buf()?;

        // handle small read buffers
        if buf.len() <= 2 * self.block_size {
            let mut outbuf = [0u8; 1024];

            let count = if data.is_empty() {
                // EOF
                let written = self.crypter.finalize(&mut outbuf)?;
                self.finalized = true;
                written
            } else {
                let mut read_size = outbuf.len() - self.block_size;
                if read_size > data.len() {
                    read_size = data.len();
                }
                let written = self.crypter.update(&data[..read_size], &mut outbuf)?;
                self.reader.consume(read_size);
                written
            };

            if count > buf.len() {
                buf.copy_from_slice(&outbuf[..buf.len()]);
                self.small_read_buf = outbuf[buf.len()..count].to_vec();
                Ok(buf.len())
            } else {
                buf[..count].copy_from_slice(&outbuf[..count]);
                Ok(count)
            }
        } else if data.is_empty() {
            // EOF
            let rest = self.crypter.finalize(buf)?;
            self.finalized = true;
            Ok(rest)
        } else {
            let mut read_size = buf.len() - self.block_size;
            if read_size > data.len() {
                read_size = data.len();
            }
            let count = self.crypter.update(&data[..read_size], buf)?;
            self.reader.consume(read_size);
            Ok(count)
        }
    }
}
