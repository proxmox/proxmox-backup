use std::future::Future;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use anyhow::{Error, Result};
use futures::{future::FutureExt, ready};
use tokio::io::AsyncWrite;
use tokio::sync::mpsc::Sender;

use proxmox::io_format_err;
use proxmox::tools::byte_buffer::ByteBuffer;
use proxmox::sys::error::io_err_other;

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

                    let mut sender = match self.sender.take() {
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
