use std::pin::Pin;
use std::task::{Context, Poll};

use anyhow::Error;
use bytes::BytesMut;
use futures::ready;
use futures::stream::{Stream, TryStream};

use pbs_datastore::Chunker;

/// Split input stream into dynamic sized chunks
pub struct ChunkStream<S: Unpin> {
    input: S,
    chunker: Chunker,
    buffer: BytesMut,
    scan_pos: usize,
}

impl<S: Unpin> ChunkStream<S> {
    pub fn new(input: S, chunk_size: Option<usize>) -> Self {
        Self {
            input,
            chunker: Chunker::new(chunk_size.unwrap_or(4 * 1024 * 1024)),
            buffer: BytesMut::new(),
            scan_pos: 0,
        }
    }
}

impl<S: Unpin> Unpin for ChunkStream<S> {}

impl<S: Unpin> Stream for ChunkStream<S>
where
    S: TryStream,
    S::Ok: AsRef<[u8]>,
    S::Error: Into<Error>,
{
    type Item = Result<BytesMut, Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        loop {
            if this.scan_pos < this.buffer.len() {
                let boundary = this.chunker.scan(&this.buffer[this.scan_pos..]);

                let chunk_size = this.scan_pos + boundary;

                if boundary == 0 {
                    this.scan_pos = this.buffer.len();
                    // continue poll
                } else if chunk_size <= this.buffer.len() {
                    let result = this.buffer.split_to(chunk_size);
                    this.scan_pos = 0;
                    return Poll::Ready(Some(Ok(result)));
                } else {
                    panic!("got unexpected chunk boundary from chunker");
                }
            }

            match ready!(Pin::new(&mut this.input).try_poll_next(cx)) {
                Some(Err(err)) => {
                    return Poll::Ready(Some(Err(err.into())));
                }
                None => {
                    this.scan_pos = 0;
                    if !this.buffer.is_empty() {
                        return Poll::Ready(Some(Ok(this.buffer.split())));
                    } else {
                        return Poll::Ready(None);
                    }
                }
                Some(Ok(data)) => {
                    this.buffer.extend_from_slice(data.as_ref());
                }
            }
        }
    }
}

/// Split input stream into fixed sized chunks
pub struct FixedChunkStream<S: Unpin> {
    input: S,
    chunk_size: usize,
    buffer: BytesMut,
}

impl<S: Unpin> FixedChunkStream<S> {
    pub fn new(input: S, chunk_size: usize) -> Self {
        Self {
            input,
            chunk_size,
            buffer: BytesMut::new(),
        }
    }
}

impl<S: Unpin> Unpin for FixedChunkStream<S> {}

impl<S: Unpin> Stream for FixedChunkStream<S>
where
    S: TryStream,
    S::Ok: AsRef<[u8]>,
{
    type Item = Result<BytesMut, S::Error>;

    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut Context,
    ) -> Poll<Option<Result<BytesMut, S::Error>>> {
        let this = self.get_mut();
        loop {
            if this.buffer.len() >= this.chunk_size {
                return Poll::Ready(Some(Ok(this.buffer.split_to(this.chunk_size))));
            }

            match ready!(Pin::new(&mut this.input).try_poll_next(cx)) {
                Some(Err(err)) => {
                    return Poll::Ready(Some(Err(err)));
                }
                None => {
                    // last chunk can have any size
                    if !this.buffer.is_empty() {
                        return Poll::Ready(Some(Ok(this.buffer.split())));
                    } else {
                        return Poll::Ready(None);
                    }
                }
                Some(Ok(data)) => {
                    this.buffer.extend_from_slice(data.as_ref());
                }
            }
        }
    }
}
