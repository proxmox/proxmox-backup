use std::collections::HashMap;

use bytes::{Bytes, BytesMut};
use failure::*;
use futures::*;

/// Trait to get digest list from index files
///
/// To allow easy iteration over all used chunks.
pub trait IndexFile: Send {
    fn index_count(&self) -> usize;
    fn index_digest(&self, pos: usize) -> Option<&[u8; 32]>;
    fn index_bytes(&self) -> u64;

    /// Returns most often used chunks
    fn find_most_used_chunks(&self, max: usize) -> HashMap<[u8; 32], usize> {
        let mut map = HashMap::new();

        for pos in 0..self.index_count() {
            let digest = self.index_digest(pos).unwrap();

            let count = map.entry(*digest).or_insert(0);
            *count += 1;
        }

        let mut most_used = Vec::new();

        for (digest, count) in map {
            if count <= 1 { continue; }
            match most_used.binary_search_by_key(&count, |&(_digest, count)| count) {
                Ok(p) => most_used.insert(p, (digest, count)),
                Err(p) => most_used.insert(p, (digest, count)),
            }

            if most_used.len() > max { let _ = most_used.pop(); }
        }

        let mut map = HashMap::new();

        for data in most_used {
            map.insert(data.0, data.1);
        }

        map
    }
}

/// Encode digest list from an `IndexFile` into a binary stream
///
/// The reader simply returns a birary stream of 32 byte digest values.
pub struct DigestListEncoder {
    index: Box<dyn IndexFile>,
    pos: usize,
    count: usize,
}

impl DigestListEncoder {

    pub fn new(index: Box<dyn IndexFile>) -> Self {
        let count = index.index_count();
        Self { index, pos: 0, count }
    }
}

impl std::io::Read for DigestListEncoder {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, std::io::Error> {
        if buf.len() < 32 {
            panic!("read buffer too small");
        }

        if self.pos < self.count {
            let mut written = 0;
            loop {
                let digest = self.index.index_digest(self.pos).unwrap();
                buf[written..(written + 32)].copy_from_slice(digest);
                self.pos += 1;
                written += 32;
                if self.pos >= self.count {
                    break;
                }
                if (written + 32) >= buf.len() {
                    break;
                }
            }
            return Ok(written);
        } else {
            return Ok(0);
        }
    }
}

/// Decodes a Stream<Item=Bytes> into Stream<Item=<[u8;32]>
///
/// The reader simply returns a birary stream of 32 byte digest values.

pub struct DigestListDecoder<S> {
    input: S,
    buffer: BytesMut,
}

impl <S> DigestListDecoder<S> {

    pub fn new(input: S) -> Self {
        Self { input, buffer: BytesMut::new() }
    }
}

impl <S> Stream for DigestListDecoder<S>
    where S: Stream<Item=Bytes>,
          S::Error: Into<Error>,
{
    type Item = [u8; 32];
    type Error = Error;

    fn poll(&mut self) -> Result<Async<Option<Self::Item>>, Self::Error> {
        loop {

            if self.buffer.len() >= 32 {

                let left = self.buffer.split_to(32);

                let mut digest = std::mem::MaybeUninit::<[u8; 32]>::uninit();
                unsafe {
                    (*digest.as_mut_ptr()).copy_from_slice(&left[..]);
                    return Ok(Async::Ready(Some(digest.assume_init())));
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
                    let rest = self.buffer.len();
                    if rest == 0 { return Ok(Async::Ready(None)); }
                    return Err(format_err!("got small digest ({} != 32).", rest));
                }
                Ok(Async::Ready(Some(data))) => {
                    self.buffer.extend_from_slice(&data);
                    // continue
                }
            }
        }
    }
}
