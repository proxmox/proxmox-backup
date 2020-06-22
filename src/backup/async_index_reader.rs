use std::future::Future;
use std::task::{Poll, Context};
use std::pin::Pin;

use anyhow::Error;
use futures::future::FutureExt;
use futures::ready;
use tokio::io::AsyncRead;

use proxmox::sys::error::io_err_other;
use proxmox::io_format_err;

use super::IndexFile;
use super::read_chunk::AsyncReadChunk;

enum AsyncIndexReaderState<S> {
    NoData,
    WaitForData(Pin<Box<dyn Future<Output = Result<(S, Vec<u8>), Error>> + Send + 'static>>),
    HaveData(usize),
}

pub struct AsyncIndexReader<S, I: IndexFile> {
    store: Option<S>,
    index: I,
    read_buffer: Vec<u8>,
    current_chunk_idx: usize,
    current_chunk_digest: [u8; 32],
    state: AsyncIndexReaderState<S>,
}

// ok because the only public interfaces operates on &mut Self
unsafe impl<S: Sync, I: IndexFile + Sync> Sync for AsyncIndexReader<S, I> {}

impl<S: AsyncReadChunk, I: IndexFile> AsyncIndexReader<S, I> {
    pub fn new(index: I, store: S) -> Self {
        Self {
            store: Some(store),
            index,
            read_buffer: Vec::with_capacity(1024*1024),
            current_chunk_idx: 0,
            current_chunk_digest: [0u8; 32],
            state: AsyncIndexReaderState::NoData,
        }
    }
}

impl<S, I> AsyncRead for AsyncIndexReader<S, I> where
S: AsyncReadChunk + Unpin + 'static,
I: IndexFile + Unpin
{
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context,
        buf: &mut [u8],
    ) -> Poll<tokio::io::Result<usize>> {
        let this = Pin::get_mut(self);
        loop {
            match &mut this.state {
                AsyncIndexReaderState::NoData => {
                    if this.current_chunk_idx >= this.index.index_count()  {
                        return Poll::Ready(Ok(0));
                    }

                    let digest = this
                        .index
                        .index_digest(this.current_chunk_idx)
                        .ok_or(io_format_err!("could not get digest"))?
                        .clone();

                    if digest  == this.current_chunk_digest {
                        this.state = AsyncIndexReaderState::HaveData(0);
                        continue;
                    }

                    this.current_chunk_digest = digest;

                    let mut store = match this.store.take() {
                        Some(store) => store,
                        None => {
                            return Poll::Ready(Err(io_format_err!("could not find store")));
                        },
                    };

                    let future = async move {
                        store.read_chunk(&digest)
                            .await
                            .map(move |x| (store, x))
                    };

                    this.state = AsyncIndexReaderState::WaitForData(future.boxed());
                },
                AsyncIndexReaderState::WaitForData(ref mut future) => {
                    match ready!(future.as_mut().poll(cx)) {
                        Ok((store, mut chunk_data)) => {
                            this.read_buffer.clear();
                            this.read_buffer.append(&mut chunk_data);
                            this.state = AsyncIndexReaderState::HaveData(0);
                            this.store = Some(store);
                        },
                        Err(err) => {
                            return Poll::Ready(Err(io_err_other(err)));
                        },
                    };
                },
                AsyncIndexReaderState::HaveData(offset) => {
                    let offset = *offset;
                    let len = this.read_buffer.len();
                    let n = if len - offset < buf.len() {
                        len - offset
                    } else {
                        buf.len()
                    };

                    buf[0..n].copy_from_slice(&this.read_buffer[offset..offset+n]);
                    if offset + n == len {
                        this.state = AsyncIndexReaderState::NoData;
                        this.current_chunk_idx += 1;
                    } else {
                        this.state = AsyncIndexReaderState::HaveData(offset + n);
                    }

                    return Poll::Ready(Ok(n));
                },
            }
        }
    }
}
