use failure::*;
use futures::*;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

pub struct ChunkInfo {
    pub digest: [u8; 32],
    pub data: bytes::BytesMut,
    pub offset: u64,
}

pub enum MergedChunkInfo {
    Known(Vec<(u64,[u8;32])>),
    New(ChunkInfo),
}

pub trait MergeKnownChunks: Sized {
    fn merge_known_chunks(self, known_chunks: Arc<Mutex<HashSet<[u8;32]>>>) -> MergeKnownChunksQueue<Self>;
}

pub struct MergeKnownChunksQueue<S> {
    input: S,
    known_chunks: Arc<Mutex<HashSet<[u8;32]>>>,
    buffer: Option<MergedChunkInfo>,
}

impl <S> MergeKnownChunks for S
    where S: Stream<Item=ChunkInfo, Error=Error>,
{
    fn merge_known_chunks(self, known_chunks: Arc<Mutex<HashSet<[u8;32]>>>) -> MergeKnownChunksQueue<Self> {
        MergeKnownChunksQueue { input: self, known_chunks, buffer: None }
    }
}

impl <S> Stream for MergeKnownChunksQueue<S>
    where S: Stream<Item=ChunkInfo, Error=Error>,
{
    type Item = MergedChunkInfo;
    type Error = Error;

    fn poll(&mut self) -> Poll<Option<MergedChunkInfo>, Error> {
        loop {
            match self.input.poll() {
                Err(err) => {
                    return Err(err);
                }
                Ok(Async::NotReady) => {
                    return Ok(Async::NotReady);
                }
                Ok(Async::Ready(None)) => {
                    if let Some(last) = self.buffer.take() {
                        return Ok(Async::Ready(Some(last)));
                    } else {
                        return Ok(Async::Ready(None));
                    }
                }
                Ok(Async::Ready(Some(chunk_info))) => {

                    let mut known_chunks = self.known_chunks.lock().unwrap();
                    let chunk_is_known = known_chunks.contains(&chunk_info.digest);

                    if chunk_is_known {

                        let last = self.buffer.take();

                        match last {
                            None => {
                                self.buffer = Some(MergedChunkInfo::Known(vec![(chunk_info.offset, chunk_info.digest)]));
                                // continue
                            }
                            Some(MergedChunkInfo::Known(mut list)) => {
                                list.push((chunk_info.offset, chunk_info.digest));
                                let len = list.len();
                                self.buffer = Some(MergedChunkInfo::Known(list));

                                if len >= 64 {
                                    return Ok(Async::Ready(self.buffer.take()));
                                }
                                // continue

                            }
                            Some(MergedChunkInfo::New(_)) => {
                                self.buffer = Some(MergedChunkInfo::Known(vec![(chunk_info.offset, chunk_info.digest)]));
                                return Ok(Async::Ready(last));
                            }
                        }

                    } else {
                        known_chunks.insert(chunk_info.digest);
                        let new = MergedChunkInfo::New(chunk_info);
                        if let Some(last) = self.buffer.take() {
                            self.buffer = Some(new);
                            return Ok(Async::Ready(Some(last)));
                        } else {
                            return Ok(Async::Ready(Some(new)));
                        }
                    }
                }
            }
        }
    }
}
