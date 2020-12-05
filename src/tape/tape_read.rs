use std::io::Read;

/// Read trait for tape devices
///
/// Normal Read, but allows to query additional status flags.
pub trait TapeRead: Read {
    /// Return true if there is an "INCOMPLETE" mark at EOF
    ///
    /// Raises an error if you query this flag before reaching EOF.
    fn is_incomplete(&self) -> Result<bool, std::io::Error>;

    /// Return true if there is a file end marker before EOF
    ///
    /// Raises an error if you query this flag before reaching EOF.
    fn has_end_marker(&self) -> Result<bool, std::io::Error>;
}

/// Read a single block from a tape device
///
/// Assumes that 'reader' is a linux tape device.
///
/// Return true on success, false on EOD
pub fn tape_device_read_block<R: Read>(
    reader: &mut R,
    buffer: &mut [u8],
) -> Result<bool, std::io::Error> {

    loop {
        match reader.read(buffer) {
            Ok(0) => { return Ok(false); /* EOD */ }
            Ok(count) => {
                if count == buffer.len() {
                    return Ok(true);
                }
                proxmox::io_bail!("short block read ({} < {}). Tape drive uses wrong block size.",
                                  count, buffer.len());
            }
            // handle interrupted system call
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => {
                continue;
            }
            Err(err) => return Err(err),
        }
    }
}
