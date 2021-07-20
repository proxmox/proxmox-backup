//! Wrappers between async readers and streams.

use std::io::{self, Read};
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use anyhow::{Error, Result};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::mpsc::Sender;
use futures::ready;
use futures::future::FutureExt;
use futures::stream::Stream;

use proxmox::io_format_err;
use proxmox::tools::byte_buffer::ByteBuffer;
use proxmox::sys::error::io_err_other;

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

/// Wrapper around tokio::sync::mpsc::Sender, which implements Write
pub struct AsyncChannelWriter {
    sender: Option<Sender<Result<Vec<u8>, Error>>>,
    buf: ByteBuffer,
    state: WriterState,
}

type SendResult = io::Result<Sender<Result<Vec<u8>>>>;

enum WriterState {
    Ready,
    Sending(Pin<Box<dyn Future<Output = SendResult> + Send + 'static>>),
}

impl AsyncChannelWriter {
    pub fn new(sender: Sender<Result<Vec<u8>, Error>>, buf_size: usize) -> Self {
        Self {
            sender: Some(sender),
            buf: ByteBuffer::with_capacity(buf_size),
            state: WriterState::Ready,
        }
    }

    fn poll_write_impl(
        &mut self,
        cx: &mut Context,
        buf: &[u8],
        flush: bool,
    ) -> Poll<io::Result<usize>> {
        loop {
            match &mut self.state {
                WriterState::Ready => {
                    if flush {
                        if self.buf.is_empty() {
                            return Poll::Ready(Ok(0));
                        }
                    } else {
                        let free_size = self.buf.free_size();
                        if free_size > buf.len() || self.buf.is_empty() {
                            let count = free_size.min(buf.len());
                            self.buf.get_free_mut_slice()[..count].copy_from_slice(&buf[..count]);
                            self.buf.add_size(count);
                            return Poll::Ready(Ok(count));
                        }
                    }

                    let sender = match self.sender.take() {
                        Some(sender) => sender,
                        None => return Poll::Ready(Err(io_err_other("no sender"))),
                    };

                    let data = self.buf.remove_data(self.buf.len()).to_vec();
                    let future = async move {
                        sender
                            .send(Ok(data))
                            .await
                            .map(move |_| sender)
                            .map_err(|err| io_format_err!("could not send: {}", err))
                    };

                    self.state = WriterState::Sending(future.boxed());
                }
                WriterState::Sending(ref mut future) => match ready!(future.as_mut().poll(cx)) {
                    Ok(sender) => {
                        self.sender = Some(sender);
                        self.state = WriterState::Ready;
                    }
                    Err(err) => return Poll::Ready(Err(err)),
                },
            }
        }
    }
}

impl AsyncWrite for AsyncChannelWriter {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context, buf: &[u8]) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        this.poll_write_impl(cx, buf, false)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        match ready!(this.poll_write_impl(cx, &[], true)) {
            Ok(_) => Poll::Ready(Ok(())),
            Err(err) => Poll::Ready(Err(err)),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context) -> Poll<io::Result<()>> {
        self.poll_flush(cx)
    }
}
