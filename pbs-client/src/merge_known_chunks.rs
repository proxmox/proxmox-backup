use std::pin::Pin;
use std::task::{Context, Poll};

use anyhow::Error;
use futures::{ready, Stream};
use pin_project_lite::pin_project;

use pbs_datastore::data_blob::ChunkInfo;

pub enum MergedChunkInfo {
    Known(Vec<(u64, [u8; 32])>),
    New(ChunkInfo),
}

pub trait MergeKnownChunks: Sized {
    fn merge_known_chunks(self) -> MergeKnownChunksQueue<Self>;
}

pin_project! {
    pub struct MergeKnownChunksQueue<S> {
        #[pin]
        input: S,
        buffer: Option<MergedChunkInfo>,
    }
}

impl<S> MergeKnownChunks for S
where
    S: Stream<Item = Result<MergedChunkInfo, Error>>,
{
    fn merge_known_chunks(self) -> MergeKnownChunksQueue<Self> {
        MergeKnownChunksQueue {
            input: self,
            buffer: None,
        }
    }
}

impl<S> Stream for MergeKnownChunksQueue<S>
where
    S: Stream<Item = Result<MergedChunkInfo, Error>>,
{
    type Item = Result<MergedChunkInfo, Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        let mut this = self.project();

        loop {
            match ready!(this.input.as_mut().poll_next(cx)) {
                Some(Err(err)) => return Poll::Ready(Some(Err(err))),
                None => {
                    if let Some(last) = this.buffer.take() {
                        return Poll::Ready(Some(Ok(last)));
                    } else {
                        return Poll::Ready(None);
                    }
                }
                Some(Ok(mergerd_chunk_info)) => {
                    match mergerd_chunk_info {
                        MergedChunkInfo::Known(list) => {
                            let last = this.buffer.take();

                            match last {
                                None => {
                                    *this.buffer = Some(MergedChunkInfo::Known(list));
                                    // continue
                                }
                                Some(MergedChunkInfo::Known(mut last_list)) => {
                                    last_list.extend_from_slice(&list);
                                    let len = last_list.len();
                                    *this.buffer = Some(MergedChunkInfo::Known(last_list));

                                    if len >= 64 {
                                        return Poll::Ready(this.buffer.take().map(Ok));
                                    }
                                    // continue
                                }
                                Some(MergedChunkInfo::New(_)) => {
                                    *this.buffer = Some(MergedChunkInfo::Known(list));
                                    return Poll::Ready(last.map(Ok));
                                }
                            }
                        }
                        MergedChunkInfo::New(chunk_info) => {
                            let new = MergedChunkInfo::New(chunk_info);
                            if let Some(last) = this.buffer.take() {
                                *this.buffer = Some(new);
                                return Poll::Ready(Some(Ok(last)));
                            } else {
                                return Poll::Ready(Some(Ok(new)));
                            }
                        }
                    }
                }
            }
        }
    }
}
