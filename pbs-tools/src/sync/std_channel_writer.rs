use std::io::Write;
use std::sync::mpsc::SyncSender;

use anyhow::{Error};

/// Wrapper around SyncSender, which implements Write
///
/// Each write in translated into a send(Vec<u8>).
pub struct StdChannelWriter(SyncSender<Result<Vec<u8>, Error>>);

impl StdChannelWriter {
    pub fn new(sender: SyncSender<Result<Vec<u8>, Error>>) -> Self {
        Self(sender)
    }
}

impl Write for StdChannelWriter {
    fn write(&mut self, buf: &[u8]) -> Result<usize, std::io::Error> {
        self.0
            .send(Ok(buf.to_vec()))
            .map_err(proxmox::sys::error::io_err_other)
            .and(Ok(buf.len()))
    }

    fn flush(&mut self) -> Result<(), std::io::Error> {
        Ok(())
    }
}
