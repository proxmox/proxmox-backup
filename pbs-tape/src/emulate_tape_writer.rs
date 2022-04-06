use std::io::{self, Write};

use crate::{BlockWrite, PROXMOX_TAPE_BLOCK_SIZE};

/// Emulate tape write behavior on a normal Writer
///
/// Data need to be written in blocks of size PROXMOX_TAPE_BLOCK_SIZE.
/// Before reaching the EOT, the writer returns ENOSPC (like a linux
/// tape device).
pub struct EmulateTapeWriter<W> {
    block_nr: usize,
    max_blocks: usize,
    writer: W,
    wrote_eof: bool,
}

impl<W: Write> EmulateTapeWriter<W> {
    /// Create a new instance allowing to write about max_size bytes
    pub fn new(writer: W, max_size: usize) -> Self {
        let mut max_blocks = max_size / PROXMOX_TAPE_BLOCK_SIZE;

        if max_blocks < 2 {
            max_blocks = 2; // at least 2 blocks
        }

        Self {
            block_nr: 0,
            wrote_eof: false,
            writer,
            max_blocks,
        }
    }
}

impl<W: Write> BlockWrite for EmulateTapeWriter<W> {
    fn write_block(&mut self, buffer: &[u8]) -> Result<bool, io::Error> {
        if buffer.len() != PROXMOX_TAPE_BLOCK_SIZE {
            proxmox_lang::io_bail!(
                "EmulateTapeWriter: got write with wrong block size ({} != {}",
                buffer.len(),
                PROXMOX_TAPE_BLOCK_SIZE
            );
        }

        if self.block_nr >= self.max_blocks + 2 {
            return Err(io::Error::from_raw_os_error(
                nix::errno::Errno::ENOSPC as i32,
            ));
        }

        self.writer.write_all(buffer)?;
        self.block_nr += 1;

        if self.block_nr > self.max_blocks {
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn write_filemark(&mut self) -> Result<(), std::io::Error> {
        if self.wrote_eof {
            proxmox_lang::io_bail!("EmulateTapeWriter: detected multiple EOF writes");
        }
        // do nothing, just record the call
        self.wrote_eof = true;
        Ok(())
    }
}
