use std::io::{self, Read};
use std::pin::Pin;
use std::task::{Context, Poll};
use std::sync::mpsc::Receiver;

use tokio::io::{AsyncRead, ReadBuf};
use futures::ready;
use futures::stream::Stream;

use pbs_runtime::block_in_place;

/// Wrapper struct to convert a Reader into a Stream
pub struct WrappedReaderStream<R: Read + Unpin> {
    reader: R,
    buffer: Vec<u8>,
}

impl <R: Read + Unpin> WrappedReaderStream<R> {

    pub fn new(reader: R) -> Self {
        let mut buffer = Vec::with_capacity(64*1024);
        unsafe { buffer.set_len(buffer.capacity()); }
        Self { reader, buffer }
    }
}

impl<R: Read + Unpin> Stream for WrappedReaderStream<R> {
    type Item = Result<Vec<u8>, io::Error>;

    fn poll_next(self: Pin<&mut Self>, _cx: &mut Context) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        match block_in_place(|| this.reader.read(&mut this.buffer)) {
            Ok(n) => {
                if n == 0 {
                    // EOF
                    Poll::Ready(None)
                } else {
                    Poll::Ready(Some(Ok(this.buffer[..n].to_vec())))
                }
            }
            Err(err) => Poll::Ready(Some(Err(err))),
        }
    }
}

/// Wrapper struct to convert an AsyncReader into a Stream
pub struct AsyncReaderStream<R: AsyncRead + Unpin> {
    reader: R,
    buffer: Vec<u8>,
}

impl <R: AsyncRead + Unpin> AsyncReaderStream<R> {

    pub fn new(reader: R) -> Self {
        let mut buffer = Vec::with_capacity(64*1024);
        unsafe { buffer.set_len(buffer.capacity()); }
        Self { reader, buffer }
    }

    pub fn with_buffer_size(reader: R, buffer_size: usize) -> Self {
        let mut buffer = Vec::with_capacity(buffer_size);
        unsafe { buffer.set_len(buffer.capacity()); }
        Self { reader, buffer }
    }
}

impl<R: AsyncRead + Unpin> Stream for AsyncReaderStream<R> {
    type Item = Result<Vec<u8>, io::Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        let mut read_buf = ReadBuf::new(&mut this.buffer);
        match ready!(Pin::new(&mut this.reader).poll_read(cx, &mut read_buf)) {
            Ok(()) => {
                let n = read_buf.filled().len();
                if n == 0 {
                    // EOF
                    Poll::Ready(None)
                } else {
                    Poll::Ready(Some(Ok(this.buffer[..n].to_vec())))
                }
            }
            Err(err) => Poll::Ready(Some(Err(err))),
        }
    }
}

/// Wrapper struct to convert a channel Receiver into a Stream
pub struct StdChannelStream<T>(pub Receiver<T>);

impl<T> Stream for StdChannelStream<T> {
    type Item = T;

    fn poll_next(self: Pin<&mut Self>, _cx: &mut Context) -> Poll<Option<Self::Item>> {
        match block_in_place(|| self.0.recv()) {
            Ok(data) => Poll::Ready(Some(data)),
            Err(_) => Poll::Ready(None),// channel closed
        }
    }
}

#[cfg(test)]
mod test {
    use std::io;

    use anyhow::Error;
    use futures::stream::TryStreamExt;

    #[test]
    fn test_wrapped_stream_reader() -> Result<(), Error> {
        pbs_runtime::main(async {
            run_wrapped_stream_reader_test().await
        })
    }

    struct DummyReader(usize);

    impl io::Read for DummyReader {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.0 += 1;

            if self.0 >= 10 {
                return Ok(0);
            }

            unsafe {
                std::ptr::write_bytes(buf.as_mut_ptr(), 0, buf.len());
            }

            Ok(buf.len())
        }
    }

    async fn run_wrapped_stream_reader_test() -> Result<(), Error> {
        let mut reader = super::WrappedReaderStream::new(DummyReader(0));
        while let Some(_data) = reader.try_next().await? {
            // just waiting
        }
        Ok(())
    }
}
