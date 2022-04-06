use endian_trait::Endian;

use crate::MediaContentHeader;

/// Write trait for tape devices
///
/// The 'write_all' function returns if the drive reached the Logical
/// End Of Media (early warning).
///
/// It is mandatory to call 'finish' before closing the stream to mark it
/// as correctly written.
///
/// Please note that there is no flush method. Tapes flush there internal
/// buffer when they write an EOF marker.
pub trait TapeWrite {
    /// writes all data, returns true on LEOM
    fn write_all(&mut self, data: &[u8]) -> Result<bool, std::io::Error>;

    /// Returns how many bytes (raw data on tape) have been written
    fn bytes_written(&self) -> usize;

    /// flush last block, write file end mark
    ///
    /// The incomplete flag is used to mark multivolume stream.
    fn finish(&mut self, incomplete: bool) -> Result<bool, std::io::Error>;

    /// Returns true if the writer already detected the logical end of media
    fn logical_end_of_media(&self) -> bool;

    /// writes header and data, returns true on LEOM
    fn write_header(
        &mut self,
        header: &MediaContentHeader,
        data: &[u8],
    ) -> Result<bool, std::io::Error> {
        if header.size as usize != data.len() {
            proxmox_lang::io_bail!("write_header with wrong size - internal error");
        }
        let header = header.to_le();

        let res = self.write_all(unsafe {
            std::slice::from_raw_parts(
                &header as *const MediaContentHeader as *const u8,
                std::mem::size_of::<MediaContentHeader>(),
            )
        })?;

        if data.is_empty() {
            return Ok(res);
        }

        self.write_all(data)
    }
}

/// Write streams of blocks
pub trait BlockWrite {
    /// Write a data block
    ///
    /// Returns true if the drive reached the Logical End Of Media
    /// (early warning)
    fn write_block(&mut self, buffer: &[u8]) -> Result<bool, std::io::Error>;

    /// Write a filemark
    fn write_filemark(&mut self) -> Result<(), std::io::Error>;
}
