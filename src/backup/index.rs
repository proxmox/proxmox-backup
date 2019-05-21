use failure::*;
use futures::*;
use bytes::{Bytes, BytesMut};

pub trait IndexFile: Send {
    fn index_count(&self) -> usize;
    fn index_digest(&self, pos: usize) -> Option<&[u8; 32]>;
}

/// This struct can read the list of chunks from an `IndexFile`
///
/// The reader simply returns a birary stream of 32 byte digest values.
pub struct ChunkListReader {
    index: Box<dyn IndexFile>,
    pos: usize,
    count: usize,
}

impl ChunkListReader {

    pub fn new(index: Box<dyn IndexFile>) -> Self {
        let count = index.index_count();
        Self { index, pos: 0, count }
    }
}

impl std::io::Read for ChunkListReader {

    fn read(&mut self, buf: &mut [u8]) -> Result<usize, std::io::Error> {
        if buf.len() < 32 { panic!("read buffer too small"); }
        if self.pos < self.count {
            let mut written = 0;
            loop {
                let digest = self.index.index_digest(self.pos).unwrap();
                unsafe { std::ptr::copy_nonoverlapping(digest.as_ptr(), buf.as_mut_ptr().add(written), 32); }
                self.pos += 1;
                written += 32;
                if self.pos >= self.count { break; }
                if (written + 32) >= buf.len() { break; }
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

                let mut digest: [u8; 32] = unsafe { std::mem::uninitialized() };
                unsafe { std::ptr::copy_nonoverlapping(left.as_ptr(), digest.as_mut_ptr(), 32); }

                return Ok(Async::Ready(Some(digest)));
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
