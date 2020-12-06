use std::io::{self, Read};

use crate::tape::file_formats::PROXMOX_TAPE_BLOCK_SIZE;

/// Emulate tape read behavior on a normal Reader
///
/// Tapes reads are always return one whole block PROXMOX_TAPE_BLOCK_SIZE.
pub struct EmulateTapeReader<R> {
    reader: R,
}

impl <R: Read> EmulateTapeReader<R> {

    pub fn new(reader: R) -> Self {
        Self { reader }
    }
}

impl <R: Read> Read for EmulateTapeReader<R> {

    fn read(&mut self, mut buffer: &mut [u8]) -> Result<usize, io::Error> {

        let initial_buffer_len = buffer.len(); // store, check later

        let mut bytes = 0;

        while !buffer.is_empty() {
            match self.reader.read(buffer) {
                Ok(0) => break,
                Ok(n) => {
                    bytes += n;
                    let tmp = buffer;
                    buffer = &mut tmp[n..];
                }
                Err(ref e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }

        if bytes == 0 {
            return Ok(0);
        }

        // test buffer len after EOF test (to allow EOF test with small buffers in BufferedReader)
        if initial_buffer_len != PROXMOX_TAPE_BLOCK_SIZE {
            proxmox::io_bail!("EmulateTapeReader: got read with wrong block size ({} != {})",
                              buffer.len(), PROXMOX_TAPE_BLOCK_SIZE);
        }

        if !buffer.is_empty() {
            Err(io::Error::new(io::ErrorKind::UnexpectedEof, "failed to fill whole buffer"))
        } else {
            Ok(bytes)
        }
    }
}
