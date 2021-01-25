use std::future::Future;
use std::task::{Poll, Context};
use std::pin::Pin;
use std::io::SeekFrom;

use anyhow::Error;
use futures::future::FutureExt;
use futures::ready;
use tokio::io::{AsyncRead, AsyncSeek, ReadBuf};

use proxmox::sys::error::io_err_other;
use proxmox::io_format_err;

use super::IndexFile;
use super::read_chunk::AsyncReadChunk;
use super::index::ChunkReadInfo;

type ReadFuture<S> = dyn Future<Output = Result<(S, Vec<u8>), Error>> + Send + 'static;

// FIXME: This enum may not be required?
// - Put the `WaitForData` case directly into a `read_future: Option<>`
// - make the read loop as follows:
//   * if read_buffer is not empty:
//        use it
//   * else if read_future is there:
//        poll it
//        if read: move data to read_buffer
//   * else
//        create read future
#[allow(clippy::enum_variant_names)]
enum AsyncIndexReaderState<S> {
    NoData,
    WaitForData(Pin<Box<ReadFuture<S>>>),
    HaveData,
}

pub struct AsyncIndexReader<S, I: IndexFile> {
    store: Option<S>,
    index: I,
    read_buffer: Vec<u8>,
    current_chunk_offset: u64,
    current_chunk_idx: usize,
    current_chunk_info: Option<ChunkReadInfo>,
    position: u64,
    seek_to_pos: i64,
    state: AsyncIndexReaderState<S>,
}

// ok because the only public interfaces operates on &mut Self
unsafe impl<S: Sync, I: IndexFile + Sync> Sync for AsyncIndexReader<S, I> {}

impl<S: AsyncReadChunk, I: IndexFile> AsyncIndexReader<S, I> {
    pub fn new(index: I, store: S) -> Self {
        Self {
            store: Some(store),
            index,
            read_buffer: Vec::with_capacity(1024 * 1024),
            current_chunk_offset: 0,
            current_chunk_idx: 0,
            current_chunk_info: None,
            position: 0,
            seek_to_pos: 0,
            state: AsyncIndexReaderState::NoData,
        }
    }
}

impl<S, I> AsyncRead for AsyncIndexReader<S, I>
where
    S: AsyncReadChunk + Unpin + Sync + 'static,
    I: IndexFile + Unpin,
{
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context,
        buf: &mut ReadBuf,
    ) -> Poll<tokio::io::Result<()>> {
        let this = Pin::get_mut(self);
        loop {
            match &mut this.state {
                AsyncIndexReaderState::NoData => {
                    let (idx, offset) = if this.current_chunk_info.is_some() &&
                        this.position == this.current_chunk_info.as_ref().unwrap().range.end
                    {
                        // optimization for sequential chunk read
                        let next_idx = this.current_chunk_idx + 1;
                        (next_idx, 0)
                    } else {
                        match this.index.chunk_from_offset(this.position) {
                            Some(res) => res,
                            None => return Poll::Ready(Ok(()))
                        }
                    };

                    if idx >= this.index.index_count() {
                        return Poll::Ready(Ok(()));
                    }

                    let info = this
                        .index
                        .chunk_info(idx)
                        .ok_or_else(|| io_format_err!("could not get digest"))?;

                    this.current_chunk_offset = offset;
                    this.current_chunk_idx = idx;
                    let old_info = this.current_chunk_info.replace(info.clone());

                    if let Some(old_info) = old_info {
                        if old_info.digest == info.digest {
                            // hit, chunk is currently in cache
                            this.state = AsyncIndexReaderState::HaveData;
                            continue;
                        }
                    }

                    // miss, need to download new chunk
                    let store = match this.store.take() {
                        Some(store) => store,
                        None => {
                            return Poll::Ready(Err(io_format_err!("could not find store")));
                        }
                    };

                    let future = async move {
                        store.read_chunk(&info.digest)
                            .await
                            .map(move |x| (store, x))
                    };

                    this.state = AsyncIndexReaderState::WaitForData(future.boxed());
                }
                AsyncIndexReaderState::WaitForData(ref mut future) => {
                    match ready!(future.as_mut().poll(cx)) {
                        Ok((store, chunk_data)) => {
                            this.read_buffer = chunk_data;
                            this.state = AsyncIndexReaderState::HaveData;
                            this.store = Some(store);
                        }
                        Err(err) => {
                            return Poll::Ready(Err(io_err_other(err)));
                        }
                    };
                }
                AsyncIndexReaderState::HaveData => {
                    let offset = this.current_chunk_offset as usize;
                    let len = this.read_buffer.len();
                    let n = if len - offset < buf.remaining() {
                        len - offset
                    } else {
                        buf.remaining()
                    };

                    buf.put_slice(&this.read_buffer[offset..(offset + n)]);
                    this.position += n as u64;

                    if offset + n == len {
                        this.state = AsyncIndexReaderState::NoData;
                    } else {
                        this.current_chunk_offset += n as u64;
                        this.state = AsyncIndexReaderState::HaveData;
                    }

                    return Poll::Ready(Ok(()));
                }
            }
        }
    }
}

impl<S, I> AsyncSeek for AsyncIndexReader<S, I>
where
    S: AsyncReadChunk + Unpin + Sync + 'static,
    I: IndexFile + Unpin,
{
    fn start_seek(
        self: Pin<&mut Self>,
        pos: SeekFrom,
    ) -> tokio::io::Result<()> {
        let this = Pin::get_mut(self);
        this.seek_to_pos = match pos {
            SeekFrom::Start(offset) => {
                offset as i64
            },
            SeekFrom::End(offset) => {
                this.index.index_bytes() as i64 + offset
            },
            SeekFrom::Current(offset) => {
                this.position as i64 + offset
            }
        };
        Ok(())
    }

    fn poll_complete(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<tokio::io::Result<u64>> {
        let this = Pin::get_mut(self);

        let index_bytes = this.index.index_bytes();
        if this.seek_to_pos < 0 {
            return Poll::Ready(Err(io_format_err!("cannot seek to negative values")));
        } else if this.seek_to_pos > index_bytes as i64 {
            this.position = index_bytes;
        } else {
            this.position = this.seek_to_pos as u64;
        }

        // even if seeking within one chunk, we need to go to NoData to
        // recalculate the current_chunk_offset (data is cached anyway)
        this.state = AsyncIndexReaderState::NoData;

        Poll::Ready(Ok(this.position))
    }
}
