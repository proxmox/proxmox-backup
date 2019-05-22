use failure::*;

use proxmox_protocol::Chunker;
use futures::{Async, Poll};
use futures::stream::Stream;

use bytes::BytesMut;

/// Split input stream into dynamic sized chunks
pub struct ChunkStream<S> {
    input: S,
    chunker: Chunker,
    buffer: BytesMut,
    scan_pos: usize,
}

impl <S> ChunkStream<S> {
    pub fn new(input: S) -> Self {
        Self { input, chunker: Chunker::new(4 * 1024 * 1024), buffer: BytesMut::new(), scan_pos: 0}
    }
}

impl <S> Stream for ChunkStream<S>
    where S: Stream,
          S::Item: AsRef<[u8]>,
          S::Error: Into<Error>,
{

    type Item = BytesMut;
    type Error = Error;

    fn poll(&mut self) -> Poll<Option<BytesMut>, Error> {
        loop {

            if self.scan_pos < self.buffer.len() {
                let boundary = self.chunker.scan(&self.buffer[self.scan_pos..]);

                let chunk_size = self.scan_pos + boundary;

                if boundary == 0 {
                    self.scan_pos = self.buffer.len();
                    // continue poll
                } else if chunk_size <= self.buffer.len() {
                    let result = self.buffer.split_to(chunk_size);
                    self.scan_pos = 0;
                    return Ok(Async::Ready(Some(result)));
                } else {
                    panic!("got unexpected chunk boundary from chunker");
                }
            }

            match self.input.poll() {
                Err(err) => {
                    return Err(err.into());
                }
                Ok(Async::NotReady) => {
                    return Ok(Async::NotReady);
                }
                Ok(Async::Ready(None)) => {
                    self.scan_pos = 0;
                    if self.buffer.len() > 0 {
                        return Ok(Async::Ready(Some(self.buffer.take())));
                    } else {
                        return Ok(Async::Ready(None));
                    }
                }
                Ok(Async::Ready(Some(data))) => {
                    self.buffer.extend_from_slice(data.as_ref());
                 }
            }
        }
    }
}

/// Split input stream into fixed sized chunks
pub struct FixedChunkStream<S> {
    input: S,
    chunk_size: usize,
    buffer: BytesMut,
}

impl <S> FixedChunkStream<S> {

    pub fn new(input: S, chunk_size: usize) -> Self {
        Self { input, chunk_size, buffer: BytesMut::new() }
    }
}

impl <S> Stream for FixedChunkStream<S>
    where S: Stream,
          S::Item: AsRef<[u8]>,
{

    type Item = BytesMut;
    type Error = S::Error;

    fn poll(&mut self) -> Poll<Option<BytesMut>, S::Error> {
        loop {

            if self.buffer.len() == self.chunk_size {
                return Ok(Async::Ready(Some(self.buffer.take())));
            } else if self.buffer.len() > self.chunk_size {
                let result = self.buffer.split_to(self.chunk_size);
                return Ok(Async::Ready(Some(result)));
            }

            match self.input.poll() {
                Err(err) => {
                    return Err(err);
                }
                Ok(Async::NotReady) => {
                    return Ok(Async::NotReady);
                }
                Ok(Async::Ready(None)) => {
                    // last chunk can have any size
                    if self.buffer.len() > 0 {
                        return Ok(Async::Ready(Some(self.buffer.take())));
                    } else {
                        return Ok(Async::Ready(None));
                    }
                }
                Ok(Async::Ready(Some(data))) => {
                    self.buffer.extend_from_slice(data.as_ref());
                }
            }
        }
    }
}
