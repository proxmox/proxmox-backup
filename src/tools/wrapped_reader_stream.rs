use std::io::{self, Read};
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio_executor::threadpool::blocking;
use futures::stream::Stream;

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
        match blocking(|| this.reader.read(&mut this.buffer)) {
            Poll::Ready(Ok(Ok(n))) => {
                if n == 0 {
                    // EOF
                    Poll::Ready(None)
                } else {
                    Poll::Ready(Some(Ok(this.buffer[..n].to_vec())))
                }
            }
            Poll::Ready(Ok(Err(err))) => Poll::Ready(Some(Err(err))),
            Poll::Ready(Err(err)) => Poll::Ready(Some(Err(io::Error::new(
                io::ErrorKind::Other,
                err.to_string(),
            )))),
            Poll::Pending => Poll::Pending,
        }
    }
}

#[cfg(test)]
mod test {
    use std::io;

    use failure::Error;
    use futures::stream::TryStreamExt;

    #[test]
    fn test_wrapped_stream_reader() -> Result<(), Error> {
        crate::tools::runtime::main(async {
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
