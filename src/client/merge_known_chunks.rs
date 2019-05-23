use failure::*;
use futures::*;
use std::collections::{VecDeque, HashSet};
use std::sync::{Arc, Mutex};

pub struct ChunkInfo {
    pub digest: [u8; 32],
    pub data: bytes::BytesMut,
    pub offset: u64,
}

pub enum MergedChunkInfo {
    Known(Vec<ChunkInfo>),
    New(ChunkInfo),
}

pub trait MergeKnownChunks: Sized {
    fn merge_known_chunks(self, known_chunks: Arc<Mutex<HashSet<[u8;32]>>>) -> MergeKnownChunksQueue<Self>;
}

pub struct MergeKnownChunksQueue<S> {
    input: S,
    known_chunks: Arc<Mutex<HashSet<[u8;32]>>>,
    queue: VecDeque<MergedChunkInfo>,
}

impl <S> MergeKnownChunks for S
    where S: Stream<Item=ChunkInfo, Error=Error>,
{
    fn merge_known_chunks(self, known_chunks: Arc<Mutex<HashSet<[u8;32]>>>) -> MergeKnownChunksQueue<Self> {
        MergeKnownChunksQueue { input: self, known_chunks, queue: VecDeque::new() }
    }
}

impl <S> Stream for MergeKnownChunksQueue<S>
    where S: Stream<Item=ChunkInfo, Error=Error>,
{
    type Item = MergedChunkInfo;
    type Error = Error;

    fn poll(&mut self) -> Poll<Option<MergedChunkInfo>, Error> {
        loop {

            if let Some(first) = self.queue.front() {
                if let MergedChunkInfo::New(_) = first {
                    return Ok(Async::Ready(self.queue.pop_front()));
                } else if self.queue.len() > 1 {
                    return Ok(Async::Ready(self.queue.pop_front()));
                } else if let MergedChunkInfo::Known(list) = first {
                    if list.len() >= 64 {
                        return Ok(Async::Ready(self.queue.pop_front()));
                    }
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
                     if let Some(item) = self.queue.pop_front() {
                         return Ok(Async::Ready(Some(item)));
                    } else {
                         return Ok(Async::Ready(None));
                    }
                }
                Ok(Async::Ready(Some(chunk_info))) => {

                    let mut known_chunks = self.known_chunks.lock().unwrap();
                    let chunk_is_known = known_chunks.contains(&chunk_info.digest);

                    if chunk_is_known {

                        if let Some(last) = self.queue.back_mut() {
                            if let MergedChunkInfo::Known(list) = last {
                                list.push(chunk_info);
                            } else {
                                let result = MergedChunkInfo::Known(vec![chunk_info]);
                                self.queue.push_back(result);
                            }
                        } else {
                            let result = MergedChunkInfo::Known(vec![chunk_info]);
                            self.queue.push_back(result);
                        }
                    } else {
                        known_chunks.insert(chunk_info.digest);
                        let result = MergedChunkInfo::New(chunk_info);
                        self.queue.push_back(result);
                    }
                }
            }
        }
    }
}
