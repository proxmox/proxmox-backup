use std::io::Write;

use tokio::task::block_in_place;

/// Wrapper around a writer which implements Write
///
/// wraps each write with a 'block_in_place' so that
/// any (blocking) writer can be safely used in async context in a
/// tokio runtime
pub struct TokioWriterAdapter<W: Write>(W);

impl<W: Write> TokioWriterAdapter<W> {
    pub fn new(writer: W) -> Self {
        Self(writer)
    }
}

impl<W: Write> Write for TokioWriterAdapter<W> {
    fn write(&mut self, buf: &[u8]) -> Result<usize, std::io::Error> {
        block_in_place(|| self.0.write(buf))
    }

    fn flush(&mut self) -> Result<(), std::io::Error> {
        block_in_place(|| self.0.flush())
    }
}
