use failure::*;
use tokio_threadpool;
use tokio::io::{AsyncRead};
use std::io::Read;
use futures::Async;
use futures::stream::Stream;
use std::io::ErrorKind::{Other, WouldBlock};

pub struct WrappedReaderStream<R: Read> {
    reader: R,
}

impl <R: Read> WrappedReaderStream<R> {

    pub fn new(reader: R) -> Self {
        Self { reader   }
    }
}

impl <R: Read> Read for WrappedReaderStream<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        //tokio::io::would_block(|| self.reader.read(buf))
        // fixme: howto??
        match tokio_threadpool::blocking(|| self.reader.read(buf)) {
            Ok(Async::Ready(res)) => res,
            Ok(Async::NotReady) =>  Err(WouldBlock.into()),
            Err(err) => Err(std::io::Error::new(Other, "`blocking` annotated I/O must be called \
                                                from the context of the Tokio runtime.")),
        }
    }
}

impl <R: Read> AsyncRead for WrappedReaderStream<R> {
    // fixme:???!!?
    unsafe fn prepare_uninitialized_buffer(&self, _: &mut [u8]) -> bool {
        false
    }
}

impl <R: Read> Stream for WrappedReaderStream<R> {

    type Item = Vec<u8>;
    type Error = std::io::Error;

    fn poll(&mut self) -> Result<Async<Option<Vec<u8>>>, std::io::Error> {
        let mut buf = [0u8;64*1024];
        match self.poll_read(&mut buf) {
            Ok(Async::Ready(n)) => {
                // By convention, if an AsyncRead says that it read 0 bytes,
                // we should assume that it has got to the end, so we signal that
                // the Stream is done in this case by returning None:
                if n == 0 {
                    Ok(Async::Ready(None))
                } else {
                    Ok(Async::Ready(Some(buf[..n].to_vec())))
                }
            },
            Ok(Async::NotReady) => Ok(Async::NotReady),
            Err(e) => Err(e)
        }
    }
}
