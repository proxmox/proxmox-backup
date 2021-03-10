use std::io::Write;

use endian_trait::Endian;

use proxmox::sys::error::SysError;

use crate::tape::file_formats::MediaContentHeader;

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
            proxmox::io_bail!("write_header with wrong size - internal error");
        }
        let header = header.to_le();

        let res = self.write_all(unsafe { std::slice::from_raw_parts(
            &header as *const MediaContentHeader as *const u8,
            std::mem::size_of::<MediaContentHeader>(),
        )})?;

        if data.is_empty() { return Ok(res); }

        self.write_all(data)
    }
}

/// Write a single block to a tape device
///
/// Assumes that 'writer' is a linux tape device.
///
/// EOM Behaviour on Linux: When the end of medium early warning is
/// encountered, the current write is finished and the number of bytes
/// is returned. The next write returns -1 and errno is set to
/// ENOSPC. To enable writing a trailer, the next write is allowed to
/// proceed and, if successful, the number of bytes is returned. After
/// this, -1 and the number of bytes are alternately returned until
/// the physical end of medium (or some other error) is encountered.
///
/// See: https://github.com/torvalds/linux/blob/master/Documentation/scsi/st.rst
///
/// On success, this returns if we en countered a EOM condition.
pub fn tape_device_write_block<W: Write>(
    writer: &mut W,
    data: &[u8],
) -> Result<bool, std::io::Error> {

    let mut leof = false;

    loop {
        match writer.write(data) {
            Ok(count) if count == data.len() => return Ok(leof),
            Ok(count) if count > 0 => {
                proxmox::io_bail!(
                    "short block write ({} < {}). Tape drive uses wrong block size.",
                    count, data.len());
            }
            Ok(_) => { // count is 0 here, assume EOT
                return Err(std::io::Error::from_raw_os_error(nix::errno::Errno::ENOSPC as i32));
            }
            // handle interrupted system call
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => {
                continue;
            }
            // detect and handle LEOM (early warning)
            Err(err) if err.is_errno(nix::errno::Errno::ENOSPC) => {
                if leof {
                    return Err(err);
                } else {
                    leof = true;
                    continue; // next write will succeed
                }
            }
            Err(err) => return Err(err),
        }
    }
}
