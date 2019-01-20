use failure::*;
use tokio_threadpool;
use std::io::Read;
use futures::Async;
use futures::stream::Stream;

pub struct WrappedReaderStream<R: Read> {
    reader: R,
}

impl <R: Read> WrappedReaderStream<R> {

    pub fn new(reader: R) -> Self {
        Self { reader   }
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
        let mut buf = [0u8;64*1024];
        match tokio_threadpool::blocking(|| self.reader.read(&mut buf)) {
            Ok(Async::Ready(Ok(n))) => {
                 if n == 0 { // EOF
                    Ok(Async::Ready(None))
                } else {
                    Ok(Async::Ready(Some(buf[..n].to_vec())))
                }
            },
            Ok(Async::Ready(Err(err))) => Err(err),
            Ok(Async::NotReady) => Ok(Async::NotReady),
            Err(err) => Err(blocking_err()),
        }
    }
}
