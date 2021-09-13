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

    /// Skip data by reading to EOF (position after EOF marker)
    ///
    // Returns the number of bytes skipped. This does not raise an
    // error if the stream has no end marker.
    fn skip_data(&mut self) -> Result<usize, std::io::Error>;
}

#[derive(thiserror::Error, Debug)]
pub enum BlockReadError {
    #[error("{0}")]
    Error(#[from] std::io::Error),
    #[error("end of file")]
    EndOfFile,
    #[error("end of data stream")]
    EndOfStream,
}

/// Read streams of blocks
pub trait BlockRead {
    /// Read the next block (whole buffer)
    fn read_block(&mut self, buffer: &mut [u8]) -> Result<usize, BlockReadError>;
}
