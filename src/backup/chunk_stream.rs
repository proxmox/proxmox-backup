use failure::*;

use proxmox_protocol::Chunker;
use futures::{Async, Poll};
use futures::stream::Stream;

/// Split input stream into dynamic sized chunks
pub struct ChunkStream<S: Stream<Item=Vec<u8>, Error=Error>> {
    input: S,
    chunker: Chunker,
    buffer: Option<Vec<u8>>,
    scan: Option<Vec<u8>>,
}

impl <S: Stream<Item=Vec<u8>, Error=Error>> ChunkStream<S> {

    pub fn new(input: S) -> Self {
        Self { input, chunker: Chunker::new(4 * 1024 * 1024), buffer: None, scan: None}
    }
}

impl <S: Stream<Item=Vec<u8>, Error=Error>> Stream for ChunkStream<S> {

    type Item = Vec<u8>;
    type Error = Error;

    fn poll(&mut self) -> Poll<Option<Vec<u8>>, Error> {
        loop {

            if let Some(data) = self.scan.take() {
                let buffer = self.buffer.get_or_insert_with(|| Vec::with_capacity(1024*1024));
                let boundary = self.chunker.scan(&data);

                if boundary == 0 {
                    buffer.extend(data);
                    // continue poll
                } else if boundary == data.len() {
                    buffer.extend(data);
                    return Ok(Async::Ready(self.buffer.take()));
                } else if boundary < data.len() {
                    let (left, right) = data.split_at(boundary);
                    buffer.extend(left);
                    self.scan = Some(right.to_vec());
                    return Ok(Async::Ready(self.buffer.take()));
                } else {
                    panic!("got unexpected chunk boundary from chunker");
                }
            }

            match self.input.poll() {
                Err(err) => {
                    return Err(err);
                }
                Ok(Async::NotReady) => {
                    return Ok(Async::NotReady);
                }
                Ok(Async::Ready(None)) => {
                    let mut data = self.buffer.take().or_else(|| Some(vec![])).unwrap();
                    if let Some(rest) = self.scan.take() { data.extend(rest); }

                    if data.len() > 0 {
                        return Ok(Async::Ready(Some(data)));
                    } else {
                        return Ok(Async::Ready(None));
                    }
                }
                Ok(Async::Ready(Some(data))) => {
                    let scan = self.scan.get_or_insert_with(|| Vec::with_capacity(1024*1024));
                    scan.extend(data);
                }
            }
        }
    }
}

use bytes::BytesMut;

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
