use failure::*;
use futures::*;

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
    fn merge_known_chunks(self) -> MergeKnownChunksQueue<Self>;
}

pub struct MergeKnownChunksQueue<S> {
    input: S,
    buffer: Option<MergedChunkInfo>,
}

impl <S> MergeKnownChunks for S
    where S: Stream<Item=MergedChunkInfo, Error=Error>,
{
    fn merge_known_chunks(self) -> MergeKnownChunksQueue<Self> {
        MergeKnownChunksQueue { input: self, buffer: None }
    }
}

impl <S> Stream for MergeKnownChunksQueue<S>
    where S: Stream<Item=MergedChunkInfo, Error=Error>,
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
                Ok(Async::Ready(Some(mergerd_chunk_info))) => {

                    match mergerd_chunk_info {
                        MergedChunkInfo::Known(list) => {

                            let last = self.buffer.take();

                            match last {
                                None => {
                                    self.buffer = Some(MergedChunkInfo::Known(list));
                                    // continue
                                }
                                Some(MergedChunkInfo::Known(mut last_list)) => {
                                    last_list.extend_from_slice(&list);
                                    let len = last_list.len();
                                    self.buffer = Some(MergedChunkInfo::Known(last_list));

                                    if len >= 64 {
                                        return Ok(Async::Ready(self.buffer.take()));
                                    }
                                    // continue
                                }
                                Some(MergedChunkInfo::New(_)) => {
                                    self.buffer = Some(MergedChunkInfo::Known(list));
                                    return Ok(Async::Ready(last));
                                }
                            }
                        }
                        MergedChunkInfo::New(chunk_info) => {
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
}
