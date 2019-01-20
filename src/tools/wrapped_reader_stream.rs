use failure::*;
use tokio_threadpool;
use std::io::Read;
use futures::Async;
use futures::stream::Stream;

pub struct WrappedReaderStream<R: Read> {
    reader: R,
    buffer: Vec<u8>,
}

impl <R: Read> WrappedReaderStream<R> {

    pub fn new(reader: R) -> Self {
        let mut buffer = Vec::with_capacity(64*1024);
        unsafe { buffer.set_len(buffer.capacity()); }
        Self { reader, buffer }
    }
}

fn blocking_err() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::Other,
        "`blocking` annotated I/O must be called from the context of the Tokio runtime.")
}

impl <R: Read> Stream for WrappedReaderStream<R> {

    type Item = Vec<u8>;
    type Error = std::io::Error;

    fn poll(&mut self) -> Result<Async<Option<Vec<u8>>>, std::io::Error> {
        match tokio_threadpool::blocking(|| self.reader.read(&mut self.buffer)) {
            Ok(Async::Ready(Ok(n))) => {
                 if n == 0 { // EOF
                    Ok(Async::Ready(None))
                } else {
                    Ok(Async::Ready(Some(self.buffer[..n].to_vec())))
                }
            },
            Ok(Async::Ready(Err(err))) => Err(err),
            Ok(Async::NotReady) => Ok(Async::NotReady),
            Err(err) => Err(blocking_err()),
        }
    }
}
