use std::io::{self, Write};

use crate::tape::file_formats::PROXMOX_TAPE_BLOCK_SIZE;

/// Emulate tape write behavior on a normal Writer
///
/// Data need to be written in blocks of size PROXMOX_TAPE_BLOCK_SIZE.
/// Before reaching the EOT, the writer returns ENOSPC (like a linux
/// tape device).
pub struct EmulateTapeWriter<W> {
    block_nr: usize,
    max_blocks: usize,
    writer: W,
    leom_sent: bool,
}

impl <W: Write> EmulateTapeWriter<W> {

    /// Create a new instance allowing to write about max_size bytes
    pub fn new(writer: W, max_size: usize) -> Self {

        let mut max_blocks = max_size/PROXMOX_TAPE_BLOCK_SIZE;

        if max_blocks < 2 {
            max_blocks = 2; // at least 2 blocks
        }

        Self {
            block_nr: 0,
            leom_sent: false,
            writer,
            max_blocks,
        }
    }
}

impl <W: Write> Write for EmulateTapeWriter<W> {

    fn write(&mut self, buffer: &[u8]) -> Result<usize, io::Error> {

        if buffer.len() != PROXMOX_TAPE_BLOCK_SIZE {
            proxmox::io_bail!("EmulateTapeWriter: got write with wrong block size ({} != {}",
                              buffer.len(), PROXMOX_TAPE_BLOCK_SIZE);
        }

        if self.block_nr >= self.max_blocks + 2 {
            return Err(io::Error::from_raw_os_error(nix::errno::Errno::ENOSPC as i32));
        }

        if self.block_nr >= self.max_blocks {
            if !self.leom_sent {
                self.leom_sent = true;
                return Err(io::Error::from_raw_os_error(nix::errno::Errno::ENOSPC as i32));
            } else {
                self.leom_sent = false;
            }
        }

        self.writer.write_all(buffer)?;
        self.block_nr += 1;

        Ok(buffer.len())
    }

    fn flush(&mut self) -> Result<(), io::Error> {
        proxmox::io_bail!("EmulateTapeWriter does not support flush");
    }
}
