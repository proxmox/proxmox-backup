use std::io::Write;

use proxmox::tools::vec;

use crate::tape::{
    TapeWrite,
    tape_device_write_block,
    file_formats::{
        BlockHeader,
        BlockHeaderFlags,
    },
};

/// Assemble and write blocks of data
///
/// This type implement 'TapeWrite'. Data written is assembled to
/// equally sized blocks (see 'BlockHeader'), which are then written
/// to the underlying writer.
pub struct BlockedWriter<W> {
    writer: W,
    buffer: Box<BlockHeader>,
    buffer_pos: usize,
    seq_nr: u32,
    logical_end_of_media: bool,
    bytes_written: usize,
}

impl <W: Write> BlockedWriter<W> {

    /// Allow access to underlying writer
    pub fn writer_ref_mut(&mut self) -> &mut W {
        &mut self.writer
    }

    /// Creates a new instance.
    pub fn new(writer: W) -> Self {
        Self {
            writer,
            buffer: BlockHeader::new(),
            buffer_pos: 0,
            seq_nr: 0,
            logical_end_of_media: false,
            bytes_written: 0,
        }
    }

    fn write_block(buffer: &BlockHeader, writer: &mut W) -> Result<bool, std::io::Error> {

        let data = unsafe {
            std::slice::from_raw_parts(
                (buffer as *const BlockHeader) as *const u8,
                BlockHeader::SIZE,
            )
        };
        tape_device_write_block(writer, data)
    }

    fn write(&mut self, data: &[u8]) -> Result<usize, std::io::Error> {

        if data.is_empty() { return Ok(0); }

        let rest = self.buffer.payload.len() - self.buffer_pos;
        let bytes = if data.len() < rest { data.len() } else { rest };
        self.buffer.payload[self.buffer_pos..(self.buffer_pos+bytes)]
            .copy_from_slice(&data[..bytes]);

        let rest = rest - bytes;

        if rest == 0 {
            self.buffer.flags = BlockHeaderFlags::empty();
            self.buffer.set_size(self.buffer.payload.len());
            self.buffer.set_seq_nr(self.seq_nr);
            self.seq_nr += 1;
            let leom = Self::write_block(&self.buffer, &mut self.writer)?;
            if leom { self.logical_end_of_media = true; }
            self.buffer_pos = 0;
            self.bytes_written += BlockHeader::SIZE;

        } else {
            self.buffer_pos = self.buffer_pos + bytes;
        }

        Ok(bytes)
    }

}

impl <W: Write> TapeWrite for BlockedWriter<W> {

    fn write_all(&mut self, mut data: &[u8]) -> Result<bool, std::io::Error> {
        while !data.is_empty() {
            match self.write(data) {
                Ok(n) => data = &data[n..],
                Err(e) => return Err(e),
            }
        }
        Ok(self.logical_end_of_media)
    }

    fn bytes_written(&self) -> usize {
        self.bytes_written
    }

    /// flush last block, set END_OF_STREAM flag
    ///
    /// Note: This may write an empty block just including the
    /// END_OF_STREAM flag.
    fn finish(&mut self, incomplete: bool) -> Result<bool, std::io::Error> {
        vec::clear(&mut self.buffer.payload[self.buffer_pos..]);
        self.buffer.flags = BlockHeaderFlags::END_OF_STREAM;
        if incomplete { self.buffer.flags |= BlockHeaderFlags::INCOMPLETE; }
        self.buffer.set_size(self.buffer_pos);
        self.buffer.set_seq_nr(self.seq_nr);
        self.seq_nr += 1;
        self.bytes_written += BlockHeader::SIZE;
        Self::write_block(&self.buffer, &mut self.writer)
    }

    /// Returns if the writer already detected the logical end of media
    fn logical_end_of_media(&self) -> bool {
        self.logical_end_of_media
    }

}
