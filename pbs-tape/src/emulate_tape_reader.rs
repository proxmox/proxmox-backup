use std::io::Read;

use proxmox_io::ReadExt;

use crate::{BlockRead, BlockReadError, PROXMOX_TAPE_BLOCK_SIZE};

/// Emulate tape read behavior on a normal Reader
///
/// Tapes reads are always return one whole block PROXMOX_TAPE_BLOCK_SIZE.
pub struct EmulateTapeReader<R: Read> {
    reader: R,
    got_eof: bool,
}

impl<R: Read> EmulateTapeReader<R> {
    pub fn new(reader: R) -> Self {
        Self {
            reader,
            got_eof: false,
        }
    }
}

impl<R: Read> BlockRead for EmulateTapeReader<R> {
    fn read_block(&mut self, buffer: &mut [u8]) -> Result<usize, BlockReadError> {
        if self.got_eof {
            return Err(BlockReadError::Error(proxmox_lang::io_format_err!(
                "detected read after EOF!"
            )));
        }
        match self.reader.read_exact_or_eof(buffer)? {
            false => {
                self.got_eof = true;
                Err(BlockReadError::EndOfFile)
            }
            true => {
                // test buffer len after EOF test (to allow EOF test with small buffers in BufferedReader)
                if buffer.len() != PROXMOX_TAPE_BLOCK_SIZE {
                    return Err(BlockReadError::Error(proxmox_lang::io_format_err!(
                        "EmulateTapeReader: read_block with wrong block size ({} != {})",
                        buffer.len(),
                        PROXMOX_TAPE_BLOCK_SIZE,
                    )));
                }
                Ok(buffer.len())
            }
        }
    }
}
